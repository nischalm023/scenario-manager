//! Within `renderer::totes`, `container_mesh` is THE BOX ITSELF: the vertices
//! and indices of an open container, in unit coordinates that one instance row
//! scales to a tote, a tray or a carton.
//!
//! It is opaque and thick-rimmed on purpose: an open top is how a viewer sees
//! what is inside, and joining the inner and outer walls at the rim is what
//! stops the interior showing through the outside. Carton flaps fold OUTSIDE
//! the opening so they never cover the contents, and a physical unit is a
//! plain cube drawn from the same mesh buffer.

use std::ops::Range;

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// 0 = container wall, 1 = carton-only flap, 2 = contents cube.
    pub part: u32,
}

pub(super) struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub shell: Range<u32>,
    pub contents: Range<u32>,
}

impl Mesh {
    fn quad(&mut self, points: [[f32; 3]; 4], normal: [f32; 3], part: u32) {
        let base = self.vertices.len() as u16;
        self.vertices.extend(points.map(|position| Vertex { position, normal, part }));
        self.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    fn flap(&mut self, points: [[f32; 3]; 4]) {
        let a = points[0];
        let u = [points[1][0] - a[0], points[1][1] - a[1], points[1][2] - a[2]];
        let v = [points[2][0] - a[0], points[2][1] - a[1], points[2][2] - a[2]];
        let normal = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        self.quad(points, normal, 1);
        self.quad([points[3], points[2], points[1], points[0]], normal.map(|x| -x), 1);
    }
}

pub(super) fn build() -> Mesh {
    let mut mesh = Mesh { vertices: Vec::new(), indices: Vec::new(), shell: 0..0, contents: 0..0 };
    let a = 0.5;
    let x = a - 0.015 / 0.65;
    let y = a - 0.015 / 0.45;
    let floor = 0.08;
    // Outer walls and underside, followed by the inner floor and walls.
    mesh.quad([[-a, -a, 0.0], [-a, a, 0.0], [a, a, 0.0], [a, -a, 0.0]], [0.0, 0.0, -1.0], 0);
    mesh.quad([[-a, -a, 0.0], [a, -a, 0.0], [a, -a, 1.0], [-a, -a, 1.0]], [0.0, -1.0, 0.0], 0);
    mesh.quad([[a, -a, 0.0], [a, a, 0.0], [a, a, 1.0], [a, -a, 1.0]], [1.0, 0.0, 0.0], 0);
    mesh.quad([[a, a, 0.0], [-a, a, 0.0], [-a, a, 1.0], [a, a, 1.0]], [0.0, 1.0, 0.0], 0);
    mesh.quad([[-a, a, 0.0], [-a, -a, 0.0], [-a, -a, 1.0], [-a, a, 1.0]], [-1.0, 0.0, 0.0], 0);
    mesh.quad([[-x, -y, floor], [x, -y, floor], [x, y, floor], [-x, y, floor]], [0.0, 0.0, 1.0], 0);
    mesh.quad([[x, -y, floor], [-x, -y, floor], [-x, -y, 1.0], [x, -y, 1.0]], [0.0, 1.0, 0.0], 0);
    mesh.quad([[x, y, floor], [x, -y, floor], [x, -y, 1.0], [x, y, 1.0]], [-1.0, 0.0, 0.0], 0);
    mesh.quad([[-x, y, floor], [x, y, floor], [x, y, 1.0], [-x, y, 1.0]], [0.0, -1.0, 0.0], 0);
    mesh.quad([[-x, -y, floor], [-x, y, floor], [-x, y, 1.0], [-x, -y, 1.0]], [1.0, 0.0, 0.0], 0);
    // Four thick top rims leave the whole centre open.
    mesh.quad([[-a, -a, 1.0], [-x, -a, 1.0], [-x, a, 1.0], [-a, a, 1.0]], [0.0, 0.0, 1.0], 0);
    mesh.quad([[x, -a, 1.0], [a, -a, 1.0], [a, a, 1.0], [x, a, 1.0]], [0.0, 0.0, 1.0], 0);
    mesh.quad([[-x, -a, 1.0], [x, -a, 1.0], [x, -y, 1.0], [-x, -y, 1.0]], [0.0, 0.0, 1.0], 0);
    mesh.quad([[-x, y, 1.0], [x, y, 1.0], [x, a, 1.0], [-x, a, 1.0]], [0.0, 0.0, 1.0], 0);
    mesh.flap([[a, -a, 1.0], [0.74, -a, 0.6], [0.74, a, 0.6], [a, a, 1.0]]);
    mesh.flap([[-0.74, -a, 0.6], [-a, -a, 1.0], [-a, a, 1.0], [-0.74, a, 0.6]]);
    mesh.flap([[-a, a, 1.0], [a, a, 1.0], [a, 0.8, 0.6], [-a, 0.8, 0.6]]);
    mesh.flap([[-a, -0.8, 0.6], [a, -0.8, 0.6], [a, -a, 1.0], [-a, -a, 1.0]]);
    mesh.shell = 0..mesh.indices.len() as u32;
    let start = mesh.indices.len() as u32;
    mesh.quad([[-a, -a, 0.0], [-a, a, 0.0], [a, a, 0.0], [a, -a, 0.0]], [0.0, 0.0, -1.0], 2);
    mesh.quad([[-a, -a, 1.0], [a, -a, 1.0], [a, a, 1.0], [-a, a, 1.0]], [0.0, 0.0, 1.0], 2);
    mesh.quad([[-a, -a, 0.0], [a, -a, 0.0], [a, -a, 1.0], [-a, -a, 1.0]], [0.0, -1.0, 0.0], 2);
    mesh.quad([[a, -a, 0.0], [a, a, 0.0], [a, a, 1.0], [a, -a, 1.0]], [1.0, 0.0, 0.0], 2);
    mesh.quad([[a, a, 0.0], [-a, a, 0.0], [-a, a, 1.0], [a, a, 1.0]], [0.0, 1.0, 0.0], 2);
    mesh.quad([[-a, a, 0.0], [-a, -a, 0.0], [-a, -a, 1.0], [-a, a, 1.0]], [-1.0, 0.0, 0.0], 2);
    mesh.contents = start..mesh.indices.len() as u32;
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_has_an_open_centre_and_flaps_stay_outside_and_below_the_rim() {
        let mesh = build();
        for face in mesh.vertices.chunks_exact(4) {
            if face[0].part == 0 && face.iter().all(|v| v.position[2] == 1.0) {
                assert!(
                    face.iter().all(|v| v.position[0] <= -0.47)
                        || face.iter().all(|v| v.position[0] >= 0.47)
                        || face.iter().all(|v| v.position[1] <= -0.46)
                        || face.iter().all(|v| v.position[1] >= 0.46)
                );
            }
            if face[0].part == 1 {
                assert!(face.iter().all(|v| v.position[0].abs() >= 0.5 || v.position[1].abs() >= 0.5));
                assert!(face.iter().all(|v| (0.0..=1.0).contains(&v.position[2])));
                assert!(face.iter().any(|v| v.position[2] < 1.0));
            }
        }
        assert_eq!(mesh.vertices.iter().filter(|v| v.part == 1).count(), 4 * 2 * 4);
        assert_eq!(mesh.contents.end - mesh.contents.start, 36);
    }

    #[test]
    fn opaque_faces_are_wound_toward_their_declared_normals() {
        let mesh = build();
        for triangle in mesh.indices.chunks_exact(3) {
            let a = mesh.vertices[triangle[0] as usize];
            let b = mesh.vertices[triangle[1] as usize];
            let c = mesh.vertices[triangle[2] as usize];
            let u = [b.position[0] - a.position[0], b.position[1] - a.position[1], b.position[2] - a.position[2]];
            let v = [c.position[0] - a.position[0], c.position[1] - a.position[1], c.position[2] - a.position[2]];
            let cross = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            assert!(cross.iter().zip(a.normal).map(|(a, b)| a * b).sum::<f32>() > 0.0);
        }
    }
}
