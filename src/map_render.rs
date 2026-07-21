use std::{sync::OnceLock, time::Duration};

use mullvad_gui_slint::model::GeoCoordinate;
use slint::{Rgb8Pixel, SharedPixelBuffer};

const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 976.0;
const FOCAL_LENGTH: f64 = 696.936_440_8;
pub const DISCONNECTED_ZOOM: f64 = 1.35;
pub const CONNECTED_ZOOM: f64 = 1.25;

const ANIMATION_MIN_TIME: f64 = 1.3;
const ANIMATION_MAX_TIME: f64 = 2.5;
const ZOOM_OUT_BREAKPOINT: f64 = 1.7;
const ZOOM_OUT_FACTOR: f64 = 1.5;
const MAX_ZOOM_OUT: f64 = DISCONNECTED_ZOOM * ZOOM_OUT_FACTOR;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MarkerState {
    #[default]
    None,
    Secure,
    Unsecure,
}

#[derive(Clone, Copy, Debug)]
struct PositionAnimation {
    start: GeoCoordinate,
    path: GeoCoordinate,
    elapsed: f64,
    duration: f64,
}

#[derive(Clone, Copy, Debug)]
enum ZoomAnimation {
    Direct {
        start: f64,
        end: f64,
        elapsed: f64,
        duration: f64,
    },
    OutIn {
        start: f64,
        middle: f64,
        end: f64,
        elapsed: f64,
        duration: f64,
    },
}

/// Stateful port of the map movement in Mullvad's `3dmap.ts`.
pub struct MapAnimator {
    coordinate: GeoCoordinate,
    target_coordinate: GeoCoordinate,
    zoom: f64,
    target_zoom: f64,
    position: Option<PositionAnimation>,
    zoom_animation: Option<ZoomAnimation>,
    marker: MarkerState,
    initialized: bool,
    dirty: bool,
}

#[derive(Clone, Copy)]
pub struct MapFrame {
    coordinate: GeoCoordinate,
    zoom: f64,
    marker_coordinate: GeoCoordinate,
    marker: MarkerState,
}

impl MapAnimator {
    pub fn new(fallback: GeoCoordinate) -> Self {
        Self {
            coordinate: fallback,
            target_coordinate: fallback,
            zoom: DISCONNECTED_ZOOM,
            target_zoom: DISCONNECTED_ZOOM,
            position: None,
            zoom_animation: None,
            marker: MarkerState::None,
            initialized: false,
            dirty: true,
        }
    }

    pub fn set_target(
        &mut self,
        coordinate: Option<GeoCoordinate>,
        zoom: f64,
        marker: MarkerState,
        animate: bool,
    ) {
        let coordinate = coordinate.unwrap_or(self.target_coordinate);
        if self.initialized
            && coordinate == self.target_coordinate
            && zoom == self.target_zoom
            && marker == self.marker
        {
            return;
        }

        self.target_coordinate = coordinate;
        self.target_zoom = zoom;
        self.marker = marker;
        if !self.initialized || !animate {
            self.coordinate = coordinate;
            self.zoom = zoom;
            self.position = None;
            self.zoom_animation = None;
            self.initialized = true;
            self.dirty = true;
            return;
        }

        if coordinate != self.coordinate {
            let path = shortest_path(self.coordinate, coordinate);
            let duration =
                (vector_length(path) / 20.0).clamp(ANIMATION_MIN_TIME, ANIMATION_MAX_TIME);
            self.position = Some(PositionAnimation {
                start: self.coordinate,
                path,
                elapsed: 0.0,
                duration,
            });
            self.zoom_animation = Some(if duration > ZOOM_OUT_BREAKPOINT {
                ZoomAnimation::OutIn {
                    start: self.zoom,
                    middle: self
                        .zoom
                        .max(zoom)
                        .mul_add(ZOOM_OUT_FACTOR, 0.0)
                        .min(MAX_ZOOM_OUT),
                    end: zoom,
                    elapsed: 0.0,
                    duration,
                }
            } else {
                ZoomAnimation::direct(self.zoom, zoom, duration)
            });
        } else {
            self.position = None;
            self.zoom_animation = Some(ZoomAnimation::direct(self.zoom, zoom, ANIMATION_MIN_TIME));
        }
        self.dirty = true;
    }

    /// Advances by wall-clock delta and returns parameters only when a frame changed.
    pub fn frame(&mut self, delta: Duration) -> Option<MapFrame> {
        let seconds = delta.as_secs_f64();
        let mut active = false;

        if let Some(mut animation) = self.position.take() {
            animation.elapsed = (animation.elapsed + seconds).min(animation.duration);
            let ratio = smooth_transition(animation.elapsed / animation.duration);
            self.coordinate = GeoCoordinate {
                latitude: animation.start.latitude + animation.path.latitude * ratio,
                longitude: normalize_longitude(
                    animation.start.longitude + animation.path.longitude * ratio,
                ),
            };
            if animation.elapsed < animation.duration {
                self.position = Some(animation);
                active = true;
            } else {
                self.coordinate = self.target_coordinate;
            }
            self.dirty = true;
        }

        if let Some(mut animation) = self.zoom_animation.take() {
            self.zoom = animation.advance(seconds);
            if animation.is_active() {
                self.zoom_animation = Some(animation);
                active = true;
            } else {
                self.zoom = self.target_zoom;
            }
            self.dirty = true;
        }

        if !self.dirty && !active {
            return None;
        }
        self.dirty = false;
        Some(MapFrame {
            coordinate: self.coordinate,
            zoom: self.zoom,
            marker_coordinate: self.target_coordinate,
            marker: self.marker,
        })
    }

    pub fn request_redraw(&mut self) {
        self.dirty = true;
    }

    #[cfg(test)]
    fn state(&self) -> (GeoCoordinate, f64, bool) {
        (
            self.coordinate,
            self.zoom,
            self.position.is_some() || self.zoom_animation.is_some(),
        )
    }
}

impl ZoomAnimation {
    fn direct(start: f64, end: f64, duration: f64) -> Self {
        Self::Direct {
            start,
            end,
            elapsed: 0.0,
            duration,
        }
    }

    fn advance(&mut self, seconds: f64) -> f64 {
        match self {
            Self::Direct {
                start,
                end,
                elapsed,
                duration,
            } => {
                *elapsed = (*elapsed + seconds).min(*duration);
                *start + smooth_transition(*elapsed / *duration) * (*end - *start)
            }
            Self::OutIn {
                start,
                middle,
                end,
                elapsed,
                duration,
            } => {
                *elapsed = (*elapsed + seconds).min(*duration);
                let ratio = *elapsed / *duration;
                if ratio <= 0.5 {
                    *start + smooth_transition(ratio * 2.0) * (*middle - *start)
                } else {
                    *middle - smooth_transition((ratio - 0.5) * 2.0) * (*middle - *end)
                }
            }
        }
    }

    fn is_active(self) -> bool {
        match self {
            Self::Direct {
                elapsed, duration, ..
            }
            | Self::OutIn {
                elapsed, duration, ..
            } => elapsed < duration,
        }
    }
}

fn smooth_transition(value: f64) -> f64 {
    0.5 - 0.5 * (value * std::f64::consts::PI).cos()
}

fn shortest_path(start: GeoCoordinate, end: GeoCoordinate) -> GeoCoordinate {
    let mut longitude = end.longitude - start.longitude;
    if longitude > 180.0 {
        longitude -= 360.0;
    } else if longitude < -180.0 {
        longitude += 360.0;
    }
    GeoCoordinate {
        latitude: end.latitude - start.latitude,
        longitude,
    }
}

fn vector_length(vector: GeoCoordinate) -> f64 {
    vector
        .latitude
        .mul_add(vector.latitude, vector.longitude * vector.longitude)
        .sqrt()
}

fn normalize_longitude(longitude: f64) -> f64 {
    (longitude + 180.0).rem_euclid(360.0) - 180.0
}

#[derive(Clone, Copy)]
struct Point3 {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Clone, Copy)]
struct ProjectedPoint {
    x: f64,
    y: f64,
    visible: bool,
}

struct MeshData {
    land_positions: Vec<Point3>,
    land_triangles: Vec<u32>,
    land_contours: Vec<u32>,
    ocean_positions: Vec<Point3>,
    ocean_triangles: Vec<u32>,
}

const SPACE: [u8; 3] = [10, 25, 35];
const OCEAN: [u8; 3] = [25, 46, 69];
const LAND: [u8; 3] = [41, 77, 115];
const SECURE: [u8; 3] = [68, 173, 77];
const UNSECURE: [u8; 3] = [227, 64, 57];

pub fn render_map(frame: MapFrame) -> SharedPixelBuffer<Rgb8Pixel> {
    let MapFrame {
        coordinate,
        zoom: distance,
        marker_coordinate,
        marker,
    } = frame;
    let mesh = mesh_data();
    let mut pixels = SharedPixelBuffer::<Rgb8Pixel>::new(WIDTH as u32, HEIGHT as u32);
    fill(pixels.make_mut_bytes(), SPACE);

    let ocean = project_vertices(&mesh.ocean_positions, coordinate, distance, 1.0);
    draw_triangles(
        pixels.make_mut_bytes(),
        &ocean,
        &mesh.ocean_triangles,
        OCEAN,
    );

    let land = project_vertices(&mesh.land_positions, coordinate, distance, 0.999_99);
    draw_triangles(pixels.make_mut_bytes(), &land, &mesh.land_triangles, LAND);
    let contour = project_vertices(&mesh.land_positions, coordinate, distance, 1.0);
    draw_contours(
        pixels.make_mut_bytes(),
        &contour,
        &mesh.land_positions,
        &mesh.land_contours,
    );

    if marker != MarkerState::None {
        draw_marker(
            pixels.make_mut_bytes(),
            coordinate,
            marker_coordinate,
            distance,
            marker,
        );
    }

    pixels
}

fn mesh_data() -> &'static MeshData {
    static MESH: OnceLock<MeshData> = OnceLock::new();
    MESH.get_or_init(|| MeshData {
        land_positions: parse_positions(include_bytes!(
            "../assets/geo/mullvad-mesh/land_positions.gl"
        )),
        land_triangles: parse_indices(include_bytes!(
            "../assets/geo/mullvad-mesh/land_triangle_indices.gl"
        )),
        land_contours: parse_indices(include_bytes!(
            "../assets/geo/mullvad-mesh/land_contour_indices.gl"
        )),
        ocean_positions: parse_positions(include_bytes!(
            "../assets/geo/mullvad-mesh/ocean_positions.gl"
        )),
        ocean_triangles: parse_indices(include_bytes!(
            "../assets/geo/mullvad-mesh/ocean_indices.gl"
        )),
    })
}

fn parse_positions(bytes: &[u8]) -> Vec<Point3> {
    bytes
        .chunks_exact(12)
        .map(|chunk| Point3 {
            x: f64::from(f32::from_le_bytes(
                chunk[0..4].try_into().expect("x coordinate"),
            )),
            y: f64::from(f32::from_le_bytes(
                chunk[4..8].try_into().expect("y coordinate"),
            )),
            z: f64::from(f32::from_le_bytes(
                chunk[8..12].try_into().expect("z coordinate"),
            )),
        })
        .collect()
}

fn parse_indices(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().expect("mesh index")))
        .collect()
}

fn project_vertices(
    positions: &[Point3],
    coordinate: GeoCoordinate,
    distance: f64,
    scale: f64,
) -> Vec<ProjectedPoint> {
    let latitude = coordinate.latitude.to_radians();
    let longitude = coordinate.longitude.to_radians();
    let (sin_latitude, cos_latitude) = latitude.sin_cos();
    let (sin_longitude, cos_longitude) = longitude.sin_cos();
    let offset = 0.088 + (distance - CONNECTED_ZOOM) * 0.3;

    positions
        .iter()
        .map(|point| {
            let x = point.x * scale;
            let y = point.y * scale;
            let z = point.z * scale;
            let rotated_x = cos_longitude * x - sin_longitude * z;
            let longitude_z = sin_longitude * x + cos_longitude * z;
            let rotated_y = cos_latitude * y - sin_latitude * longitude_z;
            let rotated_z = sin_latitude * y + cos_latitude * longitude_z;
            let depth = distance - rotated_z;
            ProjectedPoint {
                x: WIDTH / 2.0 + FOCAL_LENGTH * rotated_x / depth,
                y: HEIGHT / 2.0 - FOCAL_LENGTH * (rotated_y + offset) / depth,
                visible: distance * rotated_z - offset * rotated_y > scale * scale,
            }
        })
        .collect()
}

fn draw_triangles(pixels: &mut [u8], vertices: &[ProjectedPoint], indices: &[u32], color: [u8; 3]) {
    for triangle in indices.chunks_exact(3) {
        let Some((&a, &b, &c)) = triangle
            .first()
            .and_then(|a| vertices.get(*a as usize))
            .zip(triangle.get(1).and_then(|b| vertices.get(*b as usize)))
            .zip(triangle.get(2).and_then(|c| vertices.get(*c as usize)))
            .map(|((a, b), c)| (a, b, c))
        else {
            continue;
        };
        let area = edge(a.x, a.y, b.x, b.y, c.x, c.y);
        if area >= 0.0 {
            continue;
        }
        raster_triangle(pixels, a, b, c, color);
    }
}

fn raster_triangle(
    pixels: &mut [u8],
    a: ProjectedPoint,
    b: ProjectedPoint,
    c: ProjectedPoint,
    color: [u8; 3],
) {
    let min_x = a.x.min(b.x).min(c.x).floor().max(0.0) as i32;
    let max_x = a.x.max(b.x).max(c.x).ceil().min(WIDTH - 1.0) as i32;
    let min_y = a.y.min(b.y).min(c.y).floor().max(0.0) as i32;
    let max_y = a.y.max(b.y).max(c.y).ceil().min(HEIGHT - 1.0) as i32;
    if min_x > max_x || min_y > max_y {
        return;
    }

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let sample_x = f64::from(x) + 0.5;
            let sample_y = f64::from(y) + 0.5;
            if edge(b.x, b.y, c.x, c.y, sample_x, sample_y) <= 0.0
                && edge(c.x, c.y, a.x, a.y, sample_x, sample_y) <= 0.0
                && edge(a.x, a.y, b.x, b.y, sample_x, sample_y) <= 0.0
            {
                set_pixel(pixels, x, y, color);
            }
        }
    }
}

fn edge(ax: f64, ay: f64, bx: f64, by: f64, px: f64, py: f64) -> f64 {
    (bx - ax) * (py - ay) - (by - ay) * (px - ax)
}

fn draw_contours(
    pixels: &mut [u8],
    vertices: &[ProjectedPoint],
    positions: &[Point3],
    indices: &[u32],
) {
    for pair in indices.windows(2) {
        let Some(((a, b), (world_a, world_b))) = vertices
            .get(pair[0] as usize)
            .zip(vertices.get(pair[1] as usize))
            .zip(
                positions
                    .get(pair[0] as usize)
                    .zip(positions.get(pair[1] as usize)),
            )
        else {
            continue;
        };
        let dx = world_a.x - world_b.x;
        let dy = world_a.y - world_b.y;
        let dz = world_a.z - world_b.z;
        let world_distance_squared = dx.mul_add(dx, dy.mul_add(dy, dz * dz));
        if a.visible && b.visible && world_distance_squared < 0.0025 {
            draw_line(pixels, *a, *b, OCEAN);
        }
    }
}

fn draw_line(pixels: &mut [u8], start: ProjectedPoint, end: ProjectedPoint, color: [u8; 3]) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let steps = dx.abs().max(dy.abs()).ceil() as i32;
    if steps == 0 || steps > 4_000 {
        return;
    }
    for step in 0..=steps {
        let ratio = f64::from(step) / f64::from(steps);
        set_pixel(
            pixels,
            (start.x + dx * ratio).round() as i32,
            (start.y + dy * ratio).round() as i32,
            color,
        );
    }
}

fn draw_marker(
    pixels: &mut [u8],
    camera: GeoCoordinate,
    marker: GeoCoordinate,
    distance: f64,
    state: MarkerState,
) {
    let point = Point3 {
        x: marker.latitude.to_radians().cos() * marker.longitude.to_radians().sin(),
        y: marker.latitude.to_radians().sin(),
        z: marker.latitude.to_radians().cos() * marker.longitude.to_radians().cos(),
    };
    let projected = project_vertices(&[point], camera, distance, 1.0)[0];
    if !projected.visible {
        return;
    }
    let camera_latitude = camera.latitude.to_radians();
    let camera_longitude = camera.longitude.to_radians();
    let marker_latitude = marker.latitude.to_radians();
    let marker_longitude = marker.longitude.to_radians();
    let depth = distance
        - (camera_latitude.sin() * marker_latitude.sin()
            + camera_latitude.cos()
                * marker_latitude.cos()
                * (marker_longitude - camera_longitude).cos());
    let outer_radius = FOCAL_LENGTH * (0.015 * distance) / depth;
    let color = match state {
        MarkerState::Secure => SECURE,
        MarkerState::Unsecure => UNSECURE,
        MarkerState::None => return,
    };
    fill_circle_alpha(pixels, projected.x, projected.y, outer_radius, color, 0.4);
    fill_radial_shadow(
        pixels,
        projected.x,
        projected.y + outer_radius * 0.1,
        outer_radius * 0.56,
    );
    fill_circle_alpha(
        pixels,
        projected.x,
        projected.y,
        outer_radius * 0.37,
        [255, 255, 255],
        1.0,
    );
    fill_circle_alpha(
        pixels,
        projected.x,
        projected.y,
        outer_radius * 0.3,
        color,
        1.0,
    );
}

fn fill_circle_alpha(
    pixels: &mut [u8],
    center_x: f64,
    center_y: f64,
    radius: f64,
    color: [u8; 3],
    alpha: f64,
) {
    let min_x = (center_x - radius).floor().max(0.0) as i32;
    let max_x = (center_x + radius).ceil().min(WIDTH - 1.0) as i32;
    let min_y = (center_y - radius).floor().max(0.0) as i32;
    let max_y = (center_y + radius).ceil().min(HEIGHT - 1.0) as i32;
    let radius_squared = radius * radius;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = f64::from(x) + 0.5 - center_x;
            let dy = f64::from(y) + 0.5 - center_y;
            if dx.mul_add(dx, dy * dy) <= radius_squared {
                blend_pixel(pixels, x, y, color, alpha);
            }
        }
    }
}

fn fill_radial_shadow(pixels: &mut [u8], center_x: f64, center_y: f64, radius: f64) {
    let min_x = (center_x - radius).floor().max(0.0) as i32;
    let max_x = (center_x + radius).ceil().min(WIDTH - 1.0) as i32;
    let min_y = (center_y - radius).floor().max(0.0) as i32;
    let max_y = (center_y + radius).ceil().min(HEIGHT - 1.0) as i32;
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = f64::from(x) + 0.5 - center_x;
            let dy = f64::from(y) + 0.5 - center_y;
            let ratio = dx.mul_add(dx, dy * dy).sqrt() / radius;
            if ratio <= 1.0 {
                blend_pixel(pixels, x, y, [0, 0, 0], 0.55 * (1.0 - ratio));
            }
        }
    }
}

fn fill(pixels: &mut [u8], color: [u8; 3]) {
    for pixel in pixels.chunks_exact_mut(3) {
        pixel.copy_from_slice(&color);
    }
}

fn set_pixel(pixels: &mut [u8], x: i32, y: i32, color: [u8; 3]) {
    if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
        return;
    }
    let offset = (y as usize * WIDTH as usize + x as usize) * 3;
    pixels[offset..offset + 3].copy_from_slice(&color);
}

fn blend_pixel(pixels: &mut [u8], x: i32, y: i32, color: [u8; 3], alpha: f64) {
    if x < 0 || y < 0 || x >= WIDTH as i32 || y >= HEIGHT as i32 {
        return;
    }
    let offset = (y as usize * WIDTH as usize + x as usize) * 3;
    for channel in 0..3 {
        pixels[offset + channel] = (f64::from(pixels[offset + channel]) * (1.0 - alpha)
            + f64::from(color[channel]) * alpha)
            .round() as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOTHENBURG: GeoCoordinate = GeoCoordinate {
        latitude: 57.708_87,
        longitude: 11.974_56,
    };

    #[test]
    fn shortest_path_wraps_across_the_date_line() {
        let path = shortest_path(
            GeoCoordinate {
                latitude: 10.0,
                longitude: 170.0,
            },
            GeoCoordinate {
                latitude: 20.0,
                longitude: -170.0,
            },
        );
        assert_eq!(path.latitude, 10.0);
        assert_eq!(path.longitude, 20.0);
    }

    #[test]
    fn first_target_is_applied_without_animation() {
        let mut animator = MapAnimator::new(GOTHENBURG);
        animator.set_target(Some(GOTHENBURG), CONNECTED_ZOOM, MarkerState::Secure, true);
        let (_, zoom, active) = animator.state();
        assert_eq!(zoom, CONNECTED_ZOOM);
        assert!(!active);
    }

    #[test]
    fn repeated_target_does_not_restart_animation() {
        let mut animator = MapAnimator::new(GOTHENBURG);
        animator.set_target(
            Some(GOTHENBURG),
            DISCONNECTED_ZOOM,
            MarkerState::Unsecure,
            false,
        );
        let stockholm = GeoCoordinate {
            latitude: 59.329_3,
            longitude: 18.068_6,
        };
        animator.set_target(Some(stockholm), CONNECTED_ZOOM, MarkerState::Secure, true);
        let _ = animator.frame(Duration::from_millis(400));
        let before = animator.state();
        animator.set_target(Some(stockholm), CONNECTED_ZOOM, MarkerState::Secure, true);
        assert_eq!(animator.state(), before);
    }

    #[test]
    fn direct_zoom_uses_upstream_minimum_duration() {
        let mut animator = MapAnimator::new(GOTHENBURG);
        animator.set_target(
            Some(GOTHENBURG),
            DISCONNECTED_ZOOM,
            MarkerState::Unsecure,
            false,
        );
        animator.set_target(Some(GOTHENBURG), CONNECTED_ZOOM, MarkerState::Secure, true);
        let _ = animator.frame(Duration::from_millis(650));
        let (_, halfway_zoom, active) = animator.state();
        assert!((halfway_zoom - 1.30).abs() < 0.000_001);
        assert!(active);
        let _ = animator.frame(Duration::from_millis(650));
        let (_, final_zoom, active) = animator.state();
        assert_eq!(final_zoom, CONNECTED_ZOOM);
        assert!(!active);
    }

    #[test]
    fn official_mesh_assets_and_frame_have_expected_coverage() {
        let mesh = mesh_data();
        assert_eq!(mesh.land_positions.len(), 88_093);
        assert_eq!(mesh.land_triangles.len(), 303_615);
        assert_eq!(mesh.land_contours.len(), 98_201);
        assert_eq!(mesh.ocean_positions.len(), 2_562);
        assert_eq!(mesh.ocean_triangles.len(), 15_360);

        let pixels = render_map(MapFrame {
            coordinate: GOTHENBURG,
            zoom: DISCONNECTED_ZOOM,
            marker_coordinate: GOTHENBURG,
            marker: MarkerState::Unsecure,
        });
        let mut ocean = 0;
        let mut land = 0;
        for pixel in pixels.as_bytes().chunks_exact(3) {
            match pixel {
                [25, 46, 69] => ocean += 1,
                [41, 77, 115] => land += 1,
                _ => {}
            }
        }
        assert!(ocean > 50_000, "ocean mesh is missing");
        assert!(land > 50_000, "land mesh is missing");
    }
}
