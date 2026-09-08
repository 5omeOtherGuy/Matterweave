#!/usr/bin/env python3
"""Independent exported-source/mesh checks; no native performance qualification."""
import argparse
import hashlib
import json
import math
import struct
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


def inspect(root):
    manifest = json.loads((root / 'manifest.json').read_text())
    sources = {}
    hashes = {}
    for record in manifest['prototypes']:
        ident = record['id']
        path = root / (ident + '.snapshot.json')
        snap = json.loads(path.read_bytes())
        cells = {}
        for x, y, z, length, material in snap['runs']:
            require(length > 0 and 0 < material <= 255, 'invalid run')
            for xx in range(x, x + length):
                key = (xx, y, z)
                require(key not in cells, 'overlapping run')
                cells[key] = material
        require(len(cells) == snap['occupied_cells'] == record['occupied_cells'], 'source count')
        require(snap['scale_m'] == record['cell_size_m'], 'source scale')
        sources[ident] = (cells, snap['scale_m'])
        hashes[path.name] = hashlib.sha256(path.read_bytes()).hexdigest()
    require(sum(len(c) for c, _ in sources.values()) == manifest['counts']['unique_stored_cells'], 'unique count')
    require(sum(len(sources[i['prototype']][0]) for i in manifest['instances']) == manifest['counts']['expanded_occupied_cells'], 'expanded count')
    require(len({i['instance'] for i in manifest['instances']}) == len(manifest['instances']), 'duplicate instance')
    mesh_checks = []
    for mesh in manifest['meshes']:
        vp, ip = root / mesh['vertex_file'], root / mesh['index_file']
        vb, ib = vp.read_bytes(), ip.read_bytes()
        require(len(vb) == mesh['vertex_bytes'] == mesh['vertices'] * 36, 'vertex bytes')
        require(len(ib) == mesh['index_bytes'] == mesh['triangles'] * 12, 'index bytes')
        vertices = list(struct.iter_unpack('<9f', vb))
        indices = list(struct.iter_unpack('<3I', ib))
        require(all(math.isfinite(n) for v in vertices for n in v), 'nonfinite vertex')
        area = 0.0
        for tri in indices:
            require(all(i < len(vertices) for i in tri), 'invalid index')
            a, b, c = [vertices[i] for i in tri]
            u = [b[j] - a[j] for j in range(3)]
            v = [c[j] - a[j] for j in range(3)]
            cross = [u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0]]
            require(sum(cross[j] * a[3+j] for j in range(3)) > 0, 'winding/normal mismatch')
            area += math.sqrt(sum(n*n for n in cross)) / 2
        if mesh['lod_factor'] == 1:
            cells, scale = sources[mesh['prototype']]
            exposed = 0
            for cell in cells:
                for axis in range(3):
                    for sign in (-1, 1):
                        neighbor = list(cell)
                        neighbor[axis] += sign
                        exposed += tuple(neighbor) not in cells
            expected = exposed * scale * scale
            require(math.isclose(area, expected, rel_tol=1e-6, abs_tol=1e-6), 'source surface area mismatch')
        mesh_checks.append({'prototype':mesh['prototype'], 'lod':mesh['lod_factor'], 'surface_area_m2':area})
        hashes[vp.name] = hashlib.sha256(vb).hexdigest()
        hashes[ip.name] = hashlib.sha256(ib).hexdigest()
    composition = json.dumps(manifest['instances'], sort_keys=True, separators=(',', ':')).encode()
    return {'result':'PASS', 'scope':'host exported geometry/counts, not device or quality qualification', 'meshes':mesh_checks, 'artifact_sha256':hashes, 'composition_sha256':hashlib.sha256(composition).hexdigest()}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('gallery', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    result = inspect(args.gallery)
    text = json.dumps(result, indent=2) + '\n'
    if args.output:
        args.output.write_text(text)
    print(text)
