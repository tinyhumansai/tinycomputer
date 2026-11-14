//! Cutting, scaling, and mapping a screenshot back to the viewport.

use super::*;
use crate::agentic::captcha::pixels::Region;

/// The screenshot of a 400-pixel-wide viewport at a device pixel ratio of 2.
fn shot() -> Region {
    Region::screenshot(&png(800, 600), 400.0).unwrap()
}

#[test]
fn a_cut_maps_its_fractions_back_to_viewport_points() {
    let region = shot()
        .cut(&Rect {
            x: 50.0,
            y: 20.0,
            width: 300.0,
            height: 270.0,
        })
        .unwrap();
    assert!(near(region.point(0.0, 0.0), (50.0, 20.0)));
    assert!(near(region.point(1.0, 1.0), (350.0, 290.0)));
    assert!(near(region.point(0.5, 0.5), (200.0, 155.0)));
    assert!(near(region.point(-1.0, 2.0), (50.0, 290.0)), "clamped");
}

#[test]
fn a_cut_outside_the_screenshot_is_nothing() {
    assert!(
        shot()
            .cut(&Rect {
                x: 500.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            })
            .is_none()
    );
    assert!(Region::screenshot(b"not a png", 400.0).is_none());
    assert!(Region::screenshot(&png(10, 10), 0.0).is_none());
}

#[test]
fn cells_split_a_region_row_by_row() {
    let grid = shot()
        .cut(&Rect {
            x: 80.0,
            y: 74.0,
            width: 240.0,
            height: 162.0,
        })
        .unwrap();
    let cells = grid.cells(3, 3);
    assert_eq!(cells.len(), 9);
    assert!(near(cells[0].point(0.5, 0.5), (120.0, 101.0)));
    assert!(near(cells[4].point(0.5, 0.5), (200.0, 155.0)));
    assert!(near(cells[8].point(0.5, 0.5), (280.0, 209.0)));
}

#[test]
fn a_small_region_is_scaled_up_to_what_jev_reads() {
    let checkbox = shot()
        .cut(&Rect {
            x: 20.0,
            y: 100.0,
            width: 304.0,
            height: 78.0,
        })
        .unwrap();
    let fitted = checkbox.fit().unwrap();
    let image = image::load_from_memory(&fitted.png).unwrap();
    assert_eq!(image.height(), 280);
    assert!((1085..=1095).contains(&image.width()));
    assert!(near(checkbox.point_in(&fitted, 0.1, 0.5), (50.4, 139.0)));
}

#[test]
fn a_long_thin_region_is_scaled_down_and_padded() {
    let region = Region::screenshot(&png(3000, 200), 3000.0).unwrap();
    let fitted = region.fit().unwrap();
    let image = image::load_from_memory(&fitted.png).unwrap();
    assert_eq!((image.width(), image.height()), (1568, 280));
    // The drawn image fills the top 105 of 280 rows; the rest is padding.
    let (x, y) = region.point_in(&fitted, 0.5, 52.0 / 280.0);
    assert!((x - 1500.0).abs() < 1.0, "{x}");
    assert!((y - 99.0).abs() < 2.0, "{y}");
    let (_, below) = region.point_in(&fitted, 0.5, 0.9);
    assert!(
        (below - 200.0).abs() < f64::EPSILON,
        "padding clamps to the edge"
    );
}

#[test]
fn a_region_already_in_range_is_sent_as_it_is() {
    let region = Region::screenshot(&png(600, 400), 600.0).unwrap();
    let fitted = region.fit().unwrap();
    let image = image::load_from_memory(&fitted.png).unwrap();
    assert_eq!((image.width(), image.height()), (600, 400));
    assert!(near(region.point_in(&fitted, 0.25, 0.75), (150.0, 300.0)));
}
