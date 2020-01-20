use super::RoadGraph;
use crate::cars::roads::road_graph_synchronize::ConnectState::{First, Inactive, Unselected};
use crate::cars::roads::Intersection;
use crate::cars::{IntersectionComponent, RoadNodeComponent};
use crate::engine_interaction::{KeyCode, KeyboardInfo, MouseInfo};
use crate::interaction::{Movable, MovedEvent, Selectable, SelectedEntity};
use crate::physics::physics_components::Transform;
use crate::rendering::meshrender_component::{
    CircleRender, LineRender, LineToRender, MeshRender, MeshRenderEnum,
};
use crate::rendering::{Color, RED};
use specs::prelude::System;
use specs::prelude::*;
use specs::shred::PanicHandler;
use specs::shrev::{EventChannel, ReaderId};

#[derive(PartialEq, Eq, Clone, Copy)]
enum ConnectState {
    Inactive,
    Unselected,
    First(Entity),
}

pub struct RoadGraphSynchronize {
    reader: ReaderId<MovedEvent>,
    connect_state: ConnectState,
    show_connect: Entity,
}

impl RoadGraphSynchronize {
    pub fn new(world: &mut World) -> Self {
        <Self as System<'_>>::SystemData::setup(world);

        let reader = world
            .write_resource::<EventChannel<MovedEvent>>()
            .register_reader();

        let e = world
            .create_entity()
            .with(Transform::new([0.0, 0.0]))
            .with(MeshRender::simple(
                LineRender {
                    offset: [0.0, 0.0].into(),
                    color: RED,
                    thickness: 0.2,
                },
                9,
            ))
            .build();

        Self {
            reader,
            connect_state: Inactive,
            show_connect: e,
        }
    }
}

#[derive(SystemData)]
pub struct RGSData<'a> {
    entities: Entities<'a>,
    rg: Write<'a, RoadGraph, PanicHandler>,
    selected: Write<'a, SelectedEntity>,
    moved: Read<'a, EventChannel<MovedEvent>>,
    kbinfo: Read<'a, KeyboardInfo>,
    mouseinfo: Read<'a, MouseInfo>,
    roadnodescomponents: WriteStorage<'a, RoadNodeComponent>,
    intersections: WriteStorage<'a, IntersectionComponent>,
    meshrenders: WriteStorage<'a, MeshRender>,
    transforms: WriteStorage<'a, Transform>,
    movable: WriteStorage<'a, Movable>,
    selectable: WriteStorage<'a, Selectable>,
}

impl<'a> System<'a> for RoadGraphSynchronize {
    type SystemData = RGSData<'a>;

    fn run(&mut self, mut data: Self::SystemData) {
        // Moved events
        for event in data.moved.read(&mut self.reader) {
            if let Some(rnc) = data.roadnodescomponents.get(event.entity) {
                data.rg.set_node_position(rnc.id, event.new_pos);
            }
            if let Some(rnc) = data.intersections.get(event.entity) {
                data.rg.set_intersection_position(rnc.id, event.new_pos);
                data.rg.calculate_nodes_positions(rnc.id);
                data.rg.synchronize_positions(rnc.id, &mut data.transforms);
            }
        }

        // Intersection creation
        if data.kbinfo.just_pressed.contains(&KeyCode::I) {
            let id = data
                .rg
                .add_intersection(Intersection::new(data.mouseinfo.unprojected));
            let intersections = &data.intersections;
            if let Some(x) = data.selected.0.and_then(|x| intersections.get(x)) {
                data.rg.connect(id, x.id);
            }
            data.rg.populate_entities(
                &data.entities,
                &mut data.roadnodescomponents,
                &mut data.intersections,
                &mut data.transforms,
                &mut data.movable,
                &mut data.selectable,
            );

            *data.selected = SelectedEntity(data.rg.intersections().nodes[&id].e);
        }

        // Intersection deletion
        if data.kbinfo.just_pressed.contains(&KeyCode::Backspace) {
            if let Some(e) = data.selected.0 {
                if let Some(inter) = data.intersections.get(e) {
                    data.rg.delete_inter(inter.id, &data.entities);
                }
            }
        }

        // Connection handling
        if data.kbinfo.just_pressed.contains(&KeyCode::C) {
            self.connect_state = Unselected;
        }

        if let Some(x) = data.selected.0 {
            if let Some(interc) = data.intersections.get(x) {
                match self.connect_state {
                    Unselected => {
                        self.connect_state = First(x);
                        data.meshrenders.get_mut(self.show_connect).unwrap().hide = false;
                    }
                    First(y) => {
                        let interc2 = data.intersections.get(y).unwrap();
                        if y != x {
                            if !data.rg.intersections().is_neigh(interc.id, interc2.id) {
                                data.rg.connect(interc.id, interc2.id);
                            } else {
                                data.rg.disconnect(interc.id, interc2.id, &data.entities);
                            }
                            self.deactive_connect(&mut data);
                        }
                    }
                    _ => (),
                }
            } else {
                self.deactive_connect(&mut data);
            }
        } else {
            self.deactive_connect(&mut data);
        }

        if let First(x) = self.connect_state {
            let trans = data.transforms.get(x).unwrap().clone();
            data.transforms
                .get_mut(self.show_connect)
                .unwrap()
                .set_position(trans.position());
            match data
                .meshrenders
                .get_mut(self.show_connect)
                .and_then(|x| x.orders.get_mut(0))
            {
                Some(MeshRenderEnum::Line(x)) => {
                    x.offset = data.mouseinfo.unprojected - trans.position();
                }
                _ => (),
            }
        }

        // Population update
        if data.rg.dirty {
            data.rg.dirty = false;

            data.rg.populate_entities(
                &data.entities,
                &mut data.roadnodescomponents,
                &mut data.intersections,
                &mut data.transforms,
                &mut data.movable,
                &mut data.selectable,
            );

            {
                for (n, r) in &data.rg.nodes().nodes {
                    let e = r.e;
                    if e.is_none() {
                        continue;
                    }
                    let e = e.unwrap();

                    let mut meshb = MeshRender::simple(
                        CircleRender {
                            radius: 3.0,
                            color: Color::gray(0.5),
                            filled: true,
                            ..Default::default()
                        },
                        0,
                    );

                    for nei in data.rg.nodes().get_neighs(*n) {
                        let e_nei = data.rg.nodes().nodes[&nei.to].e;
                        if e_nei.is_none() {
                            continue;
                        }
                        let e_nei = e_nei.unwrap();
                        meshb.add(LineToRender {
                            color: Color::gray(0.5),
                            to: e_nei,
                            thickness: 6.0,
                        });
                    }

                    data.meshrenders
                        .insert(e, meshb)
                        .expect("Error inserting mesh for graph");
                }
            }

            {
                for (n, r) in &data.rg.intersections().nodes {
                    let e = r.e;
                    if e.is_none() {
                        continue;
                    }
                    let e = e.unwrap();

                    let mut meshb = MeshRender::simple(
                        CircleRender {
                            radius: 2.0,
                            color: RED,
                            filled: true,
                            ..Default::default()
                        },
                        1,
                    );

                    for nei in data.rg.intersections().get_neighs(*n) {
                        let e_nei = data.rg.intersections().nodes[&nei.to].e;
                        if e_nei.is_none() {
                            continue;
                        }
                        let e_nei = e_nei.unwrap();
                        meshb.add(LineToRender {
                            color: RED,
                            to: e_nei,
                            thickness: 0.1,
                        });
                    }

                    data.meshrenders
                        .insert(e, meshb)
                        .expect("Error inserting mesh for graph");
                }
            }
        }
    }
}

impl RoadGraphSynchronize {
    fn deactive_connect(&mut self, data: &mut RGSData) {
        self.connect_state = Inactive;
        data.meshrenders.get_mut(self.show_connect).unwrap().hide = true;
    }
}
