//! Authorizes requests and consumes grants before dialing approved addresses.

use std::{future::Future, io, net::SocketAddr, sync::Arc, time::Duration};

use tokio::net::TcpStream;

use super::{
    BrowserDialTarget, DialGrant, EgressAuthorizer,
    forward::{
        bad_gateway, forward_request_body_with_lease, forward_response_with_lease,
        initialize_upstream, refuse, tunnel_with_lease, validate_lease, validation_interval,
    },
    request::{has_valid_buffered_body, read_proxy_request},
    upstream::{RoutedLease, UpstreamProxy, open_tunnel},
};

pub(super) async fn serve(
    client: TcpStream,
    auth: Arc<dyn EgressAuthorizer>,
    session_id: String,
    proxy_password: String,
    upstreams: Arc<[UpstreamProxy]>,
) -> io::Result<()> {
    Box::pin(serve_with_connector(
        client,
        auth,
        session_id,
        proxy_password,
        &upstreams,
        TcpStream::connect,
    ))
    .await
}

pub(super) async fn serve_with_connector<C, F>(
    mut client: TcpStream,
    auth: Arc<dyn EgressAuthorizer>,
    session_id: String,
    proxy_password: String,
    upstreams: &[UpstreamProxy],
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
    let route = upstreams
        .iter()
        .find(|proxy| proxy.routes(&request.target.hostname));
    let (mut upstream, lease, auth) = if let Some(proxy) = route {
        match connect_through_upstream(&auth, &request.target, first_grant, proxy, &mut connect)
            .await
        {
            Routed::Open(upstream, lease, routed) => (upstream, lease, routed),
            Routed::Refused => return refuse(&mut client).await,
            Routed::Gateway => return bad_gateway(&mut client).await,
        }
    } else {
        let Some((upstream, lease)) =
            connect_approved_addresses(&auth, &request.target, first_grant, &mut connect).await
        else {
            return refuse(&mut client).await;
        };
        (upstream, lease, auth)
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

/// How a routed connection ended up.
enum Routed {
    /// A tunnel to the destination, its upstream proxy lease, and the
    /// authorizer that validates both leases.
    Open(TcpStream, String, Arc<dyn EgressAuthorizer>),
    /// The host refused the upstream proxy dial, or it could not be made.
    Refused,
    /// The upstream proxy answered but would not open the tunnel.
    Gateway,
}

/// Reach an authorized destination through its upstream proxy. The
/// destination grant was issued by the host's policy check; it is spent here
/// for this connection, and the only socket opened is to an address the host
/// granted for the upstream proxy itself.
async fn connect_through_upstream<C, F>(
    auth: &Arc<dyn EgressAuthorizer>,
    target: &BrowserDialTarget,
    destination_grant: DialGrant,
    proxy: &UpstreamProxy,
    connect: &mut C,
) -> Routed
where
    C: FnMut(SocketAddr) -> F,
    F: Future<Output = io::Result<TcpStream>>,
{
    let proxy_target = proxy.dial_target(&target.session_id);
    let Some(proxy_grant) = authorize(auth, &proxy_target).await else {
        log::warn!("browser upstream proxy dial was not authorized by the host");
        return Routed::Refused;
    };
    let Some(destination_address) = destination_grant.addresses.first().copied() else {
        return Routed::Refused;
    };
    let destination_lease = destination_grant.lease.clone();
    if !matches!(
        tokio::time::timeout(
            Duration::from_secs(5),
            auth.consume(target.clone(), destination_grant, destination_address),
        )
        .await,
        Ok(Ok(()))
    ) {
        return Routed::Refused;
    }
    let Some((mut upstream, proxy_lease)) =
        connect_approved_addresses(auth, &proxy_target, proxy_grant, connect).await
    else {
        log::warn!("browser upstream proxy could not be reached");
        return Routed::Refused;
    };
    if let Err(error) = open_tunnel(&mut upstream, proxy, target).await {
        log::warn!("browser upstream proxy did not open a tunnel: {error}");
        return Routed::Gateway;
    }
    let routed: Arc<dyn EgressAuthorizer> = Arc::new(RoutedLease {
        inner: auth.clone(),
        destination_lease,
    });
    Routed::Open(upstream, proxy_lease, routed)
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
