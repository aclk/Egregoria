use crate::gui::Tool;
use crate::input::{MouseButton, MouseInfo};
use crate::rendering::immediate::{ImmediateDraw, ImmediateSound};
use crate::uiworld::UiWorld;
use common::AudioKind;
use egregoria::engine_interaction::{WorldCommand, WorldCommands};
use egregoria::Egregoria;
use geom::{vec2, PolyLine3, Spline3, Vec2, AABB};
use geom::{Camera, Spline};
use map_model::{Intersection, LanePatternBuilder, Map, MapProject, ProjectKind, PylonPosition};
use BuildState::{Hover, Interpolation, Start};
use ProjectKind::{Building, Ground, Inter, Road};

const MAX_TURN_ANGLE: f32 = 30.0 * std::f32::consts::PI / 180.0;

#[derive(Copy, Clone, Debug)]
pub enum BuildState {
    Hover,
    Start(MapProject),
    Interpolation(Vec2, MapProject),
}

impl Default for BuildState {
    fn default() -> Self {
        BuildState::Hover
    }
}

register_resource_noserialize!(RoadBuildResource);
#[derive(Default)]
pub struct RoadBuildResource {
    pub build_state: BuildState,
    pub pattern_builder: LanePatternBuilder,
    pub snap_to_grid: bool,
    pub height_offset: f32,
}

#[profiling::function]
pub fn roadbuild(goria: &Egregoria, uiworld: &mut UiWorld) {
    let state = &mut *uiworld.write::<RoadBuildResource>();
    let immdraw = &mut *uiworld.write::<ImmediateDraw>();
    let immsound = &mut *uiworld.write::<ImmediateSound>();
    let mouseinfo = uiworld.read::<MouseInfo>();
    let tool = uiworld.read::<Tool>();
    let map = &*goria.map();
    let commands: &mut WorldCommands = &mut *uiworld.commands();
    let cam = &*uiworld.read::<Camera>();

    if !matches!(*tool, Tool::RoadbuildStraight | Tool::RoadbuildCurved) {
        state.build_state = BuildState::Hover;
        return;
    }

    let unproj = unwrap_ret!(mouseinfo.unprojected);
    let grid_size = 15.0;
    let mousepos = if state.snap_to_grid {
        let v = unproj.xy().snap(grid_size, grid_size);
        v.z(unwrap_ret!(map.terrain.height(v)) + 0.3 + state.height_offset)
    } else {
        unproj.up(0.3 + state.height_offset)
    };

    let log_camheight = cam.eye().z.log10();
    let cutoff = inline_tweak::tweak!(3.3);

    if state.snap_to_grid && log_camheight < cutoff {
        let alpha = 1.0 - log_camheight / cutoff;
        let col = common::config().gui_primary.a(alpha);
        let screen = AABB::new(unproj.xy(), unproj.xy()).expand(300.0);
        let startx = (screen.ll.x / grid_size).ceil() * grid_size;
        let starty = (screen.ll.y / grid_size).ceil() * grid_size;

        let height = |p| map.terrain.height(p);
        for x in 0..(screen.w() / grid_size) as i32 {
            let x = startx + x as f32 * grid_size;
            for y in 0..(screen.h() / grid_size) as i32 {
                let y = starty + y as f32 * grid_size;
                let p = vec2(x, y);
                let p3 = p.z(unwrap_cont!(height(p)) + 0.1);
                let px = p + Vec2::x(grid_size);
                let py = p + Vec2::y(grid_size);

                immdraw
                    .line(p3, px.z(unwrap_cont!(height(px)) + 0.1), 0.3)
                    .color(col);
                immdraw
                    .line(p3, py.z(unwrap_cont!(height(py)) + 0.1), 0.3)
                    .color(col);
            }
        }
    }

    for command in uiworld.received_commands().iter() {
        if matches!(
            *uiworld.read::<Tool>(),
            Tool::RoadbuildCurved | Tool::RoadbuildStraight
        ) {
            if let WorldCommand::MapMakeConnection(_, to, _, _) = command {
                let proj = map.project(to.pos, 0.0);
                if let Some(
                    proj
                    @
                    MapProject {
                        kind: ProjectKind::Inter(_),
                        ..
                    },
                ) = proj
                {
                    state.build_state = BuildState::Start(proj);
                }
            }
        }
    }

    if mouseinfo.just_pressed.contains(&MouseButton::Right) {
        state.build_state = BuildState::Hover;
    }

    let mut cur_proj = unwrap_ret!(map.project(mousepos, 0.0));
    if matches!(cur_proj.kind, ProjectKind::Lot(_)) {
        cur_proj.kind = ProjectKind::Ground;
    }

    let patwidth = state.pattern_builder.width();

    if let ProjectKind::Road(r_id) = cur_proj.kind {
        let r = &map.roads()[r_id];
        if r.points
            .first()
            .is_close(cur_proj.pos, r.interface_from(r.src) + patwidth * 0.5)
        {
            cur_proj = MapProject {
                kind: ProjectKind::Inter(r.src),
                pos: r.points.first(),
            };
        } else if r
            .points
            .last()
            .is_close(cur_proj.pos, r.interface_from(r.dst) + patwidth * 0.5)
        {
            cur_proj = MapProject {
                kind: ProjectKind::Inter(r.dst),
                pos: r.points.last(),
            };
        }
    }

    if matches!(*tool, Tool::RoadbuildCurved) {
        if let Start(proj) = state.build_state {
            cur_proj = MapProject {
                pos: mousepos.xy().z(proj.pos.z),
                kind: ProjectKind::Ground,
            };
        }
    }
    let is_valid = match (state.build_state, cur_proj.kind) {
        (Hover, Building(_)) => false,
        (Start(selected_proj), _) => {
            compatible(map, cur_proj, selected_proj)
                && check_angle(map, selected_proj, cur_proj.pos.xy())
                && check_angle(map, cur_proj, selected_proj.pos.xy())
        }
        (Interpolation(interpoint, selected_proj), _) => {
            let sp = Spline {
                from: selected_proj.pos.xy(),
                to: cur_proj.pos.xy(),
                from_derivative: (interpoint - selected_proj.pos.xy())
                    * std::f32::consts::FRAC_1_SQRT_2,
                to_derivative: (cur_proj.pos.xy() - interpoint) * std::f32::consts::FRAC_1_SQRT_2,
            };

            compatible(map, cur_proj, selected_proj)
                && check_angle(map, selected_proj, interpoint)
                && check_angle(map, cur_proj, interpoint)
                && !sp.is_steep(state.pattern_builder.width())
        }
        _ => true,
    };

    state.update_drawing(map, immdraw, cur_proj, patwidth, is_valid);

    if is_valid && mouseinfo.just_pressed.contains(&MouseButton::Left) {
        log::info!(
            "left clicked with state {:?} and {:?}",
            state.build_state,
            cur_proj.kind
        );

        // FIXME: Use or patterns when stable
        match (state.build_state, cur_proj.kind, *tool) {
            (Hover, Ground, _) | (Hover, Road(_), _) | (Hover, Inter(_), _) => {
                // Hover selection
                state.build_state = Start(cur_proj);
            }
            (Start(v), Ground, Tool::RoadbuildCurved) => {
                // Set interpolation point
                state.build_state = Interpolation(mousepos.xy(), v);
            }
            (Start(selected_proj), _, _) => {
                // Straight connection to something
                immsound.play("road_lay", AudioKind::Ui);
                commands.map_make_connection(
                    selected_proj,
                    cur_proj,
                    None,
                    state.pattern_builder.build(),
                );

                state.build_state = Hover;
            }
            (Interpolation(interpoint, selected_proj), _, _) => {
                // Interpolated connection to something
                immsound.play("road_lay", AudioKind::Ui);
                commands.map_make_connection(
                    selected_proj,
                    cur_proj,
                    Some(interpoint),
                    state.pattern_builder.build(),
                );

                state.build_state = Hover;
            }
            _ => {}
        }
    }
}

fn check_angle(map: &Map, from: MapProject, to: Vec2) -> bool {
    match from.kind {
        Inter(i) => {
            let inter = &map.intersections()[i];
            let dir = (to - inter.pos.xy()).normalize();
            for &road in &inter.roads {
                let road = &map.roads()[road];
                let v = road.dir_from(i);
                if v.angle(dir).abs() < MAX_TURN_ANGLE {
                    return false;
                }
            }
            true
        }
        Road(r) => {
            let r = &map.roads()[r]; // fixme don't crash
            let (proj, _, rdir1) = r.points().project_segment_dir(from.pos);
            let rdir2 = -rdir1;
            let dir = (to - proj.xy()).normalize();
            if rdir1.xy().angle(dir).abs() < MAX_TURN_ANGLE
                || rdir2.xy().angle(dir).abs() < MAX_TURN_ANGLE
            {
                return false;
            }
            true
        }
        _ => true,
    }
}

fn compatible(map: &Map, x: MapProject, y: MapProject) -> bool {
    // enforce at most 18 deg angle
    if x.pos.distance(y.pos) < 10.0
        || (x.pos.z - y.pos.z).abs() > 0.2 * x.pos.xy().distance(y.pos.xy())
    {
        return false;
    }
    match (x.kind, y.kind) {
        (Ground, Ground)
        | (Ground, Road(_))
        | (Ground, Inter(_))
        | (Road(_), Ground)
        | (Inter(_), Ground) => true,
        (Road(id), Road(id2)) => id != id2,
        (Inter(id), Inter(id2)) => id != id2,
        (Inter(id_inter), Road(id_road)) | (Road(id_road), Inter(id_inter)) => {
            let r = &map.roads()[id_road];
            r.src != id_inter && r.dst != id_inter
        }
        _ => false,
    }
}

impl RoadBuildResource {
    pub fn update_drawing(
        &self,
        map: &Map,
        immdraw: &mut ImmediateDraw,
        proj: MapProject,
        patwidth: f32,
        is_valid: bool,
    ) {
        let mut proj_pos = proj.pos;
        proj_pos.z += 0.1;
        let col = if is_valid {
            common::config().gui_primary
        } else {
            common::config().gui_danger
        };

        let interf = |ang: Vec2, proj: MapProject| match proj.kind {
            Inter(i) => map
                .intersections()
                .get(i)
                .map(|i| i.interface_at(map.roads(), patwidth, ang))
                .unwrap_or_default(),
            Road(_) => Intersection::empty_interface(patwidth),
            Building(_) => 0.0,
            ProjectKind::Lot(_) => 0.0,
            Ground => Intersection::empty_interface(patwidth),
        };

        let p = match self.build_state {
            BuildState::Hover => {
                immdraw.circle(proj_pos, patwidth * 0.5).color(col);
                return;
            }
            BuildState::Start(x) => {
                immdraw.circle(proj_pos, patwidth * 0.5).color(col);
                immdraw.circle(x.pos.up(0.1), patwidth * 0.5).color(col);
                immdraw.line(proj_pos, x.pos.up(0.1), patwidth).color(col);
                let istart = interf((proj_pos - x.pos).xy().normalize(), x);
                let iend = interf(-(proj_pos - x.pos).xy().normalize(), proj);
                PolyLine3::new(vec![x.pos.up(0.1), proj_pos]).cut(istart, iend)
            }
            BuildState::Interpolation(p, x) => {
                let sp = Spline3 {
                    from: x.pos.up(0.1),
                    to: proj_pos,
                    from_derivative: (p - x.pos.xy()).z0() * std::f32::consts::FRAC_1_SQRT_2,
                    to_derivative: (proj_pos.xy() - p).z0() * std::f32::consts::FRAC_1_SQRT_2,
                };
                let points: Vec<_> = sp.smart_points(1.0, 0.0, 1.0).collect();

                immdraw.polyline(&*points, patwidth).color(col);

                immdraw.circle(sp.get(0.0), patwidth * 0.5).color(col);
                immdraw.circle(sp.get(1.0), patwidth * 0.5).color(col);

                let istart = interf((p - x.pos.xy()).normalize(), x);
                let iend = interf(-(proj_pos.xy() - p).normalize(), proj);

                PolyLine3::new(points).cut(istart, iend)
            }
        };

        for PylonPosition {
            terrain_height,
            pos,
            ..
        } in map_model::Road::pylons_positions(&p, &map.terrain)
        {
            immdraw
                .circle(pos.xy().z(terrain_height + 0.1), patwidth * 0.5)
                .color(col);
        }
    }
}
