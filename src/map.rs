use std::{cell::Cell, f64::consts::PI, rc::Rc};

use gtk::prelude::*;
use mullvad_gtk::model::{GeoCoordinate, TunnelStatus};
use serde_json::Value;

const DEFAULT_COORDINATE: GeoCoordinate = GeoCoordinate {
    latitude: 57.7089,
    longitude: 11.9746,
};

#[derive(Clone, Copy)]
enum MarkerState {
    Hidden,
    Unsecured,
    Securing,
    Secured,
}

struct MapState {
    latitude: Cell<f64>,
    longitude: Cell<f64>,
    target_latitude: Cell<f64>,
    target_longitude: Cell<f64>,
    pulse: Cell<f64>,
    marker: Cell<MarkerState>,
}

#[derive(Clone)]
pub struct MapView {
    pub widget: gtk::DrawingArea,
    state: Rc<MapState>,
}

impl MapView {
    pub fn new() -> Self {
        let geometry = Rc::new(parse_geometry());
        let state = Rc::new(MapState {
            latitude: Cell::new(DEFAULT_COORDINATE.latitude),
            longitude: Cell::new(DEFAULT_COORDINATE.longitude),
            target_latitude: Cell::new(DEFAULT_COORDINATE.latitude),
            target_longitude: Cell::new(DEFAULT_COORDINATE.longitude),
            pulse: Cell::new(0.0),
            marker: Cell::new(MarkerState::Hidden),
        });
        let widget = gtk::DrawingArea::builder()
            .content_width(390)
            .content_height(556)
            .hexpand(true)
            .vexpand(true)
            .build();
        widget.add_css_class("map");
        widget.set_draw_func({
            let geometry = Rc::clone(&geometry);
            let state = Rc::clone(&state);
            move |_area, context, width, height| {
                draw_map(
                    context,
                    f64::from(width),
                    f64::from(height),
                    &geometry,
                    &state,
                );
            }
        });
        widget.add_tick_callback({
            let state = Rc::clone(&state);
            move |area, _clock| {
                state.latitude.set(interpolate(
                    state.latitude.get(),
                    state.target_latitude.get(),
                ));
                state.longitude.set(interpolate(
                    state.longitude.get(),
                    state.target_longitude.get(),
                ));
                state.pulse.set((state.pulse.get() + 0.025) % 1.0);
                area.queue_draw();
                gtk::glib::ControlFlow::Continue
            }
        });

        Self { widget, state }
    }

    pub fn set_status(&self, status: &TunnelStatus) {
        if let Some(coordinate) = status.coordinates() {
            self.state.target_latitude.set(coordinate.latitude);
            self.state.target_longitude.set(coordinate.longitude);
        }
        self.state.marker.set(match status {
            TunnelStatus::Connected { .. } => MarkerState::Secured,
            TunnelStatus::Connecting { .. } | TunnelStatus::Disconnecting => MarkerState::Securing,
            TunnelStatus::Disconnected { .. } | TunnelStatus::Error(_) => MarkerState::Unsecured,
            TunnelStatus::Unavailable(_) => MarkerState::Hidden,
        });
    }
}

fn interpolate(current: f64, target: f64) -> f64 {
    let delta = target - current;
    if delta.abs() < 0.01 {
        target
    } else {
        current + delta * 0.035
    }
}

fn draw_map(
    context: &gtk::cairo::Context,
    width: f64,
    height: f64,
    geometry: &[Vec<(f64, f64)>],
    state: &MapState,
) {
    context.set_source_rgb(10.0 / 255.0, 25.0 / 255.0, 35.0 / 255.0);
    context.paint().expect("map background should draw");

    let radius = width.max(height) * 0.58;
    let center_x = width * 0.5;
    let center_y = height * 0.42;
    context.arc(center_x, center_y, radius, 0.0, PI * 2.0);
    context.set_source_rgb(25.0 / 255.0, 46.0 / 255.0, 69.0 / 255.0);
    context.fill_preserve().expect("map ocean should draw");
    context.clip();

    context.set_source_rgb(41.0 / 255.0, 77.0 / 255.0, 115.0 / 255.0);
    for ring in geometry {
        let mut visible_points = 0;
        for &(longitude, latitude) in ring {
            if let Some((x, y)) = project(
                longitude,
                latitude,
                state.longitude.get(),
                state.latitude.get(),
                radius,
            ) {
                if visible_points == 0 {
                    context.move_to(center_x + x, center_y + y);
                } else {
                    context.line_to(center_x + x, center_y + y);
                }
                visible_points += 1;
            } else if visible_points > 2 {
                context.close_path();
                context.fill().expect("map land should draw");
                visible_points = 0;
            } else {
                context.new_path();
                visible_points = 0;
            }
        }
        if visible_points > 2 {
            context.close_path();
            context.fill().expect("map land should draw");
        } else {
            context.new_path();
        }
    }

    context.reset_clip();
    draw_marker(context, center_x, center_y, radius, state);
}

fn project(
    longitude: f64,
    latitude: f64,
    center_longitude: f64,
    center_latitude: f64,
    radius: f64,
) -> Option<(f64, f64)> {
    let longitude = (longitude - center_longitude).to_radians();
    let latitude = latitude.to_radians();
    let center_latitude = center_latitude.to_radians();
    let visibility = center_latitude.sin() * latitude.sin()
        + center_latitude.cos() * latitude.cos() * longitude.cos();
    if visibility < 0.0 {
        return None;
    }
    let x = radius * latitude.cos() * longitude.sin();
    let y = -radius
        * (center_latitude.cos() * latitude.sin()
            - center_latitude.sin() * latitude.cos() * longitude.cos());
    Some((x, y))
}

fn draw_marker(context: &gtk::cairo::Context, x: f64, y: f64, radius: f64, state: &MapState) {
    let (red, green, blue) = match state.marker.get() {
        MarkerState::Hidden => return,
        MarkerState::Secured => (68.0 / 255.0, 173.0 / 255.0, 77.0 / 255.0),
        MarkerState::Securing => (1.0, 213.0 / 255.0, 36.0 / 255.0),
        MarkerState::Unsecured => (227.0 / 255.0, 64.0 / 255.0, 57.0 / 255.0),
    };
    let pulse = if matches!(state.marker.get(), MarkerState::Securing) {
        state.pulse.get()
    } else {
        0.35
    };
    let outer_radius = radius * (0.035 + pulse * 0.025);
    context.arc(x, y, outer_radius, 0.0, PI * 2.0);
    context.set_source_rgba(red, green, blue, 0.35 * (1.0 - pulse));
    context.fill().expect("map marker pulse should draw");
    context.arc(x, y, radius * 0.022, 0.0, PI * 2.0);
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.fill().expect("map marker ring should draw");
    context.arc(x, y, radius * 0.016, 0.0, PI * 2.0);
    context.set_source_rgb(red, green, blue);
    context.fill().expect("map marker should draw");
}

fn parse_geometry() -> Vec<Vec<(f64, f64)>> {
    let value: Value = serde_json::from_str(include_str!("../assets/geo/countries.geo.json"))
        .expect("embedded Mullvad geography should be valid JSON");
    let mut rings = Vec::new();
    let Some(features) = value.get("features").and_then(Value::as_array) else {
        return rings;
    };
    for feature in features {
        let Some(geometry) = feature.get("geometry") else {
            continue;
        };
        let Some(coordinates) = geometry.get("coordinates") else {
            continue;
        };
        match geometry.get("type").and_then(Value::as_str) {
            Some("Polygon") => collect_polygon(coordinates, &mut rings),
            Some("MultiPolygon") => {
                if let Some(polygons) = coordinates.as_array() {
                    for polygon in polygons {
                        collect_polygon(polygon, &mut rings);
                    }
                }
            }
            _ => {}
        }
    }
    rings
}

fn collect_polygon(value: &Value, rings: &mut Vec<Vec<(f64, f64)>>) {
    let Some(polygon) = value.as_array() else {
        return;
    };
    for ring in polygon {
        let Some(points) = ring.as_array() else {
            continue;
        };
        let points = points
            .iter()
            .filter_map(|point| {
                let point = point.as_array()?;
                Some((point.first()?.as_f64()?, point.get(1)?.as_f64()?))
            })
            .collect::<Vec<_>>();
        if points.len() > 2 {
            rings.push(points);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_upstream_geography_contains_country_rings() {
        let geometry = parse_geometry();
        assert!(geometry.len() > 100);
        assert!(geometry.iter().all(|ring| ring.len() > 2));
    }

    #[test]
    fn orthographic_projection_hides_the_far_side() {
        assert!(project(0.0, 0.0, 0.0, 0.0, 100.0).is_some());
        assert!(project(180.0, 0.0, 0.0, 0.0, 100.0).is_none());
    }
}
