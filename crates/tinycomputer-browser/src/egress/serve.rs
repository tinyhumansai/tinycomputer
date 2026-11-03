//! Authorizes requests and consumes grants before dialing approved addresses.

use std::{future::Future, io, net::SocketAddr, sync::Arc, time::Duration};

use tokio::net::TcpStream;

use super::{
    BrowserDialTarget, DialGrant, EgressAuthorizer,
    forward::{
        forward_request_body_with_lease, forward_response_with_lease, initialize_upstream, refuse,
        tunnel_with_lease, validate_lease, validation_interval,
    },
    request::{has_valid_buffered_body, read_proxy_request},
};

pub(super) async fn serve(
    client: TcpStream,
    auth: Arc<dyn EgressAuthorizer>,
    session_id: String,
    proxy_password: String,
) -> io::Result<()> {
    Box::pin(serve_with_connector(
        client,
        auth,
        session_id,
        proxy_password,
        TcpStream::connect,
    ))
    .await
}

pub(super) async fn serve_with_connector<C, F>(
    mut client: TcpStream,
    auth: Arc<dyn EgressAuthorizer>,
    session_id: String,
    proxy_password: String,
    mut connect: C,
) -> io::Result<()>
where
    C: FnMut(SocketAddr) -> F,
    F: Future<Output = io::Result<TcpStream>>,
{
    let Some(request) = read_proxy_request(&mut client, &session_id, &proxy_password).await? else {
        return Ok(());
    };
    if !has_valid_buffered_body(&request) {
        return refuse(&mut client).await;
    }
    let Some(first_grant) = authorize(&auth, &request.target).await else {
        return refuse(&mut client).await;
    };
    let Some((mut upstream, lease)) =
        connect_approved_addresses(&auth, &request.target, first_grant, &mut connect).await
    else {
        return refuse(&mut client).await;
    };
    let validations = validation_interval();
    if !validate_lease(&auth, &lease).await? {
        return Ok(());
    }
    if !initialize_upstream(
        &mut client,
        &mut upstream,
        &request,
        &auth,
        &lease,
        &validations,
    )
    .await?
    {
        return Ok(());
    }
    if request.connect || request.has_upgrade {
        tunnel_with_lease(&mut client, &mut upstream, &auth, lease, &validations).await?;
    } else {
        let (mut client_reader, mut client_writer) = client.into_split();
        let (mut upstream_reader, mut upstream_writer) = upstream.into_split();
        let response = forward_response_with_lease(
            &mut client_writer,
            &mut upstream_reader,
            &auth,
            lease.clone(),
            &validations,
        );
        tokio::pin!(response);
        tokio::select! {
            result = &mut response => result?,
            upload = forward_request_body_with_lease(
                &mut client_reader,
                &mut upstream_writer,
                &request,
                &auth,
                &lease,
                &validations,
            ) => {
                if upload? {
                    response.await?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn connect_approved_addresses<C, F>(
    auth: &Arc<dyn EgressAuthorizer>,
    target: &BrowserDialTarget,
    first_grant: DialGrant,
    connect: &mut C,
) -> Option<(TcpStream, String)>
where
    C: FnMut(SocketAddr) -> F,
    F: Future<Output = io::Result<TcpStream>>,
{
    let addresses = first_grant.addresses.clone();
    let mut connected = None;
    for (index, address) in addresses.into_iter().enumerate() {
        // A consume call spends its one-use grant. Reauthorize every later
        // candidate so a failed dial cannot reuse a spent authorization.
        let grant = if index == 0 {
            first_grant.clone()
        } else {
            match authorize(auth, target).await {
                Some(grant) if grant.addresses.contains(&address) => grant,
                _ => continue,
            }
        };
        if !matches!(
            tokio::time::timeout(
                Duration::from_secs(5),
                auth.consume(target.clone(), grant.clone(), address),
            )
            .await,
            Ok(Ok(()))
        ) {
            return None;
        }
        if let Ok(Ok(upstream)) =
            tokio::time::timeout(Duration::from_secs(5), connect(address)).await
        {
            connected = Some((upstream, grant.lease));
            break;
        }
    }
    connected
}

async fn authorize(
    auth: &Arc<dyn EgressAuthorizer>,
    target: &BrowserDialTarget,
) -> Option<DialGrant> {
    match tokio::time::timeout(Duration::from_secs(5), auth.authorize(target.clone())).await {
        Ok(Ok(grant)) if !grant.addresses.is_empty() && grant.addresses.len() <= 64 => Some(grant),
        _ => None,
    }
}
