#!/usr/bin/env node

/**
 * Bake a vendor HTM model into the low-poly proxy desvis instances per bot.
 *
 * The vendor GLB is a 21 MB Draco-compressed 760k-triangle asset — far too
 * heavy to commit and far too heavy to draw once per bot per frame. This
 * script decodes it once, offline, and emits the few-thousand-triangle
 * `worldview/worldview-render/assets/htm-mesh.bin` that the renderer actually ships. The GLB
 * itself stays out of the repo; re-run this only when the vendor model changes.
 *
 * Usage:
 *   npm install draco3d          # one-off; not a repo dependency
 *   node scripts/bake-htm-mesh.js <model.glb> [cluster_mm]
 *
 * Output format (little-endian), consumed by
 * `worldview/worldview-render/src/renderer/htm_mesh.rs`:
 *   magic  "HTMM"                    4 bytes
 *   u32    version = 2
 *   u32    vertex_count
 *   u32    index_count
 *   per vertex, 28 bytes             f32 x 3 position, f32 x 3 normal,
 *                                    u8 x 4 sRGB colour (alpha always 255)
 *   u16 x index_count                triangle list, zero-padded to 4 bytes
 */

const fs = require('fs');
const path = require('path');
const draco3d = require('draco3d');

// Footprint of the drawn bot, in metres: the shader's `0.35 * cell_size`
// radius doubled, so the model's long axis spans the old disc's diameter and
// bots keep the footprint operators are used to.
const TARGET_LENGTH_M = 0.7;
// Vertex-clustering cell edge in millimetres. Coarser = fewer triangles; at
// on-screen bot sizes anything below ~10 mm is invisible detail.
const DEFAULT_CLUSTER_MM = 20;
// Bytes per emitted vertex: position xyz, normal xyz, sRGB colour rgba.
const VERTEX_BYTES = 3 * 4 + 3 * 4 + 4;

const SRC = process.argv[2];
const CLUSTER_MM = Number(process.argv[3] || DEFAULT_CLUSTER_MM);
const OUT = path.join(__dirname, '../worldview/worldview-render/assets/htm-mesh.bin');

if (!SRC) {
  console.error('usage: node scripts/bake-htm-mesh.js <model.glb> [cluster_mm]');
  process.exit(2);
}

/**
 * Pull every Draco-compressed primitive out of a GLB as one merged triangle
 * soup, painting each primitive's vertices with its material's base colour.
 *
 * A glTF primitive carries exactly one material, so the base colour is the
 * only paint the vendor file offers without decoding its 15 MB of textures —
 * and at bot-on-screen sizes a flat colour per part is all that reads anyway.
 * Materials that name no `baseColorFactor` take glTF's white default.
 */
function decodeGlb(decoderModule, file) {
  const data = fs.readFileSync(file);
  if (data.slice(0, 4).toString('ascii') !== 'glTF') throw new Error('not a GLB');
  const jsonLen = data.readUInt32LE(12);
  const json = JSON.parse(data.slice(20, 20 + jsonLen).toString('utf8'));
  const bin = data.slice(20 + jsonLen + 8);

  const decoder = new decoderModule.Decoder();
  const positions = [];
  const colors = [];
  const indices = [];

  for (const mesh of json.meshes) {
    for (const prim of mesh.primitives) {
      const ext = prim.extensions && prim.extensions.KHR_draco_mesh_compression;
      if (!ext) throw new Error('primitive is not Draco-compressed; this baker only handles Draco');
      const bv = json.bufferViews[ext.bufferView];
      const slice = bin.slice(bv.byteOffset || 0, (bv.byteOffset || 0) + bv.byteLength);

      const buf = new decoderModule.DecoderBuffer();
      buf.Init(new Int8Array(slice), slice.length);
      const dmesh = new decoderModule.Mesh();
      const status = decoder.DecodeBufferToMesh(buf, dmesh);
      if (!status.ok()) throw new Error(`Draco decode failed: ${status.error_msg()}`);

      const base = positions.length / 3;
      const attr = decoder.GetAttributeByUniqueId(dmesh, ext.attributes.POSITION);
      const arr = new decoderModule.DracoFloat32Array();
      decoder.GetAttributeFloatForAllPoints(dmesh, attr, arr);
      for (let i = 0; i < dmesh.num_points() * 3; i++) positions.push(arr.GetValue(i));
      decoderModule.destroy(arr);

      const material = (json.materials || [])[prim.material] || {};
      const paint = ((material.pbrMetallicRoughness || {}).baseColorFactor) || [1, 1, 1, 1];
      for (let i = 0; i < dmesh.num_points(); i++) colors.push(paint[0], paint[1], paint[2]);

      const face = new decoderModule.DracoInt32Array();
      for (let f = 0; f < dmesh.num_faces(); f++) {
        decoder.GetFaceFromMesh(dmesh, f, face);
        indices.push(base + face.GetValue(0), base + face.GetValue(1), base + face.GetValue(2));
      }
      decoderModule.destroy(face);
      decoderModule.destroy(dmesh);
      decoderModule.destroy(buf);
    }
  }
  decoderModule.destroy(decoder);
  return {
    positions: new Float32Array(positions),
    colors: new Float32Array(colors),
    indices: new Uint32Array(indices),
  };
}

/**
 * Rewrite vendor coordinates into the shader's bot-local frame, in metres:
 * +X forward, +Y left, +Z up, origin at the centre of the footprint with the
 * wheels on z = 0. The vendor file is glTF's Y-up with the model's long axis
 * on Z and its lit control panel at +Z, so forward is +Z there. Taking
 * (x, y, z)_local = (z, x, y)_vendor is a cyclic axis rotation, which keeps
 * the mesh right-handed — triangle winding and normals carry over untouched.
 */
function toBotLocal(positions) {
  const n = positions.length / 3;
  const lo = [Infinity, Infinity, Infinity];
  const hi = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < n; i++) {
    for (let a = 0; a < 3; a++) {
      const v = positions[i * 3 + a];
      if (v < lo[a]) lo[a] = v;
      if (v > hi[a]) hi[a] = v;
    }
  }
  const scale = TARGET_LENGTH_M / (hi[2] - lo[2]);
  const cx = (lo[0] + hi[0]) / 2;
  const cz = (lo[2] + hi[2]) / 2;

  const out = new Float32Array(n * 3);
  for (let i = 0; i < n; i++) {
    out[i * 3 + 0] = (positions[i * 3 + 2] - cz) * scale;
    out[i * 3 + 1] = (positions[i * 3 + 0] - cx) * scale;
    out[i * 3 + 2] = (positions[i * 3 + 1] - lo[1]) * scale;
  }
  return out;
}

/**
 * Collapse the mesh onto a uniform grid: every vertex in a cell becomes one
 * output vertex at that cell's average position and average colour, and
 * triangles whose corners land in fewer than three distinct cells disappear.
 * Crude next to a quadric-error simplifier, but it is deterministic, needs no
 * connectivity, and survives the vendor mesh's many disconnected shells. A
 * cell straddling two parts averages their paints, which softens the seam
 * rather than making one part swallow the other.
 */
function clusterDecimate(positions, colors, indices, cell) {
  const n = positions.length / 3;
  const key = new Int32Array(n);
  const cells = new Map();
  const sum = [];
  const paintSum = [];
  const count = [];

  for (let i = 0; i < n; i++) {
    const gx = Math.floor(positions[i * 3 + 0] / cell);
    const gy = Math.floor(positions[i * 3 + 1] / cell);
    const gz = Math.floor(positions[i * 3 + 2] / cell);
    const k = `${gx},${gy},${gz}`;
    let id = cells.get(k);
    if (id === undefined) {
      id = cells.size;
      cells.set(k, id);
      sum.push(0, 0, 0);
      paintSum.push(0, 0, 0);
      count.push(0);
    }
    key[i] = id;
    for (let a = 0; a < 3; a++) {
      sum[id * 3 + a] += positions[i * 3 + a];
      paintSum[id * 3 + a] += colors[i * 3 + a];
    }
    count[id] += 1;
  }

  const verts = new Float32Array(cells.size * 3);
  const paints = new Float32Array(cells.size * 3);
  for (let c = 0; c < cells.size; c++) {
    for (let a = 0; a < 3; a++) {
      verts[c * 3 + a] = sum[c * 3 + a] / count[c];
      paints[c * 3 + a] = paintSum[c * 3 + a] / count[c];
    }
  }

  // Keep one triangle per distinct corner triple; the vendor mesh stacks many
  // near-coincident shells that would otherwise collapse onto each other.
  const seen = new Set();
  const tris = [];
  for (let t = 0; t < indices.length; t += 3) {
    const a = key[indices[t]];
    const b = key[indices[t + 1]];
    const c = key[indices[t + 2]];
    if (a === b || b === c || a === c) continue;
    const lo = Math.min(a, b, c);
    const hi = Math.max(a, b, c);
    const k = `${lo},${a + b + c - lo - hi},${hi}`;
    if (seen.has(k)) continue;
    seen.add(k);
    tris.push(a, b, c);
  }
  return { verts, paints, tris: new Uint32Array(tris) };
}

/** Drop clusters no surviving triangle refers to, and renumber. */
function compact(verts, paints, tris) {
  const remap = new Int32Array(verts.length / 3).fill(-1);
  let next = 0;
  for (const t of tris) if (remap[t] < 0) remap[t] = next++;
  const outV = new Float32Array(next * 3);
  const outP = new Float32Array(next * 3);
  for (let i = 0; i < remap.length; i++) {
    if (remap[i] < 0) continue;
    for (let a = 0; a < 3; a++) {
      outV[remap[i] * 3 + a] = verts[i * 3 + a];
      outP[remap[i] * 3 + a] = paints[i * 3 + a];
    }
  }
  const outI = new Uint32Array(tris.length);
  for (let i = 0; i < tris.length; i++) outI[i] = remap[tris[i]];
  return { verts: outV, paints: outP, tris: outI };
}

/** Area-weighted vertex normals recomputed from the decimated triangles. */
function computeNormals(verts, tris) {
  const nrm = new Float32Array(verts.length);
  for (let t = 0; t < tris.length; t += 3) {
    const [i, j, k] = [tris[t] * 3, tris[t + 1] * 3, tris[t + 2] * 3];
    const ux = verts[j] - verts[i], uy = verts[j + 1] - verts[i + 1], uz = verts[j + 2] - verts[i + 2];
    const vx = verts[k] - verts[i], vy = verts[k + 1] - verts[i + 1], vz = verts[k + 2] - verts[i + 2];
    // Un-normalised cross product weights each face by twice its area.
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    for (const b of [i, j, k]) {
      nrm[b] += nx;
      nrm[b + 1] += ny;
      nrm[b + 2] += nz;
    }
  }
  for (let i = 0; i < nrm.length; i += 3) {
    const len = Math.hypot(nrm[i], nrm[i + 1], nrm[i + 2]);
    if (len < 1e-12) {
      nrm[i] = 0; nrm[i + 1] = 0; nrm[i + 2] = 1;
    } else {
      nrm[i] /= len; nrm[i + 1] /= len; nrm[i + 2] /= len;
    }
  }
  return nrm;
}

/**
 * glTF states base colours as linear light; desvis's shaders write straight
 * into a non-sRGB framebuffer whose bytes egui already fills with gamma-
 * encoded values. Encoding here — rather than in the shader — is also what
 * makes eight bits enough for this model's very dark greys.
 */
function srgbByte(linear) {
  const encoded = linear <= 0.0031308 ? 12.92 * linear : 1.055 * Math.pow(linear, 1 / 2.4) - 0.055;
  return Math.max(0, Math.min(255, Math.round(255 * encoded)));
}

draco3d.createDecoderModule({}).then((decoderModule) => {
  const raw = decodeGlb(decoderModule, SRC);
  console.log(`decoded ${raw.positions.length / 3} verts / ${raw.indices.length / 3} tris`);

  const local = toBotLocal(raw.positions);
  const clustered = clusterDecimate(local, raw.colors, raw.indices, CLUSTER_MM / 1000);
  const { verts, paints, tris } = compact(clustered.verts, clustered.paints, clustered.tris);
  if (verts.length / 3 > 0xffff) throw new Error(`${verts.length / 3} verts exceeds the u16 index range`);
  const nrm = computeNormals(verts, tris);

  const vcount = verts.length / 3;
  const icount = tris.length;
  const idxBytes = Math.ceil(icount * 2 / 4) * 4;
  const buf = Buffer.alloc(16 + vcount * VERTEX_BYTES + idxBytes);
  buf.write('HTMM', 0, 'ascii');
  buf.writeUInt32LE(2, 4);
  buf.writeUInt32LE(vcount, 8);
  buf.writeUInt32LE(icount, 12);
  for (let i = 0; i < vcount; i++) {
    const o = 16 + i * VERTEX_BYTES;
    for (let a = 0; a < 3; a++) buf.writeFloatLE(verts[i * 3 + a], o + a * 4);
    for (let a = 0; a < 3; a++) buf.writeFloatLE(nrm[i * 3 + a], o + 12 + a * 4);
    for (let a = 0; a < 3; a++) buf.writeUInt8(srgbByte(paints[i * 3 + a]), o + 24 + a);
    buf.writeUInt8(255, o + 27);
  }
  for (let i = 0; i < icount; i++) buf.writeUInt16LE(tris[i], 16 + vcount * VERTEX_BYTES + i * 2);

  fs.writeFileSync(OUT, buf);
  const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < vcount; i++) {
    for (let a = 0; a < 3; a++) {
      lo[a] = Math.min(lo[a], verts[i * 3 + a]);
      hi[a] = Math.max(hi[a], verts[i * 3 + a]);
    }
  }
  console.log(`baked ${vcount} verts / ${icount / 3} tris -> ${OUT} (${buf.length} bytes)`);
  console.log(`bot-local extent m: x ${lo[0].toFixed(3)}..${hi[0].toFixed(3)}  ` +
    `y ${lo[1].toFixed(3)}..${hi[1].toFixed(3)}  z ${lo[2].toFixed(3)}..${hi[2].toFixed(3)}`);
});
