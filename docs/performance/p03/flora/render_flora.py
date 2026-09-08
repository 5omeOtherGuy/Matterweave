"""Render trusted exported flora meshes for HOST geometry inspection only."""
import argparse,json
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw
from mesh_views import view


def mesh(root, record):
    vertices=np.fromfile(root/record['vertex_file'],dtype='<f4').reshape(-1,9)
    faces=np.fromfile(root/record['index_file'],dtype='<u4').reshape(-1,3)
    return vertices[:,:3],vertices[:,6:9],faces


def render(root):
    manifest=json.loads((root/'manifest.json').read_text());out=root/'views';out.mkdir(exist_ok=True)
    angles=[('Front',(0,0.15,1),False),('Side',(1,0.15,0),False),('Underside',(0.4,-1,0.6),False),('Above',(0.6,1,0.8),False),('Silhouette',(0,0,1),True),('Side silhouette',(1,0,0),True)]
    by_id={}
    for prototype in manifest['prototypes']:
        ident=prototype['id'];records=sorted([m for m in manifest['meshes'] if m['prototype']==ident],key=lambda r:r['lod_factor'])
        levels=[mesh(root,record) for record in records];by_id[ident]=levels[0]
        if 'terrain' in ident:continue
        v,c,f=levels[0];canvas=Image.new('RGB',(1440,1020),(25,31,39));draw=ImageDraw.Draw(canvas)
        for i,(label,angle,silhouette) in enumerate(angles):
            x=i%3*480;y=i//3*510;canvas.paste(view(v,c,f,angle,silhouette=silhouette),(x,y+30));draw.text((x+12,y+10),ident+' / '+label,fill='white')
        canvas.save(out/(ident+'-source.png'))
        reference=np.concatenate([v for v,c,f in levels]);canvas=Image.new('RGB',(480*len(levels),510),(25,31,39));draw=ImageDraw.Draw(canvas)
        for i,((v,c,f),record) in enumerate(zip(levels,records)):
            canvas.paste(view(v,c,f,(0.5,0.2,1),reference=reference),(480*i,30));draw.text((480*i+12,10),'LOD '+str(record['lod_factor'])+' / identical framing',fill='white')
        canvas.save(out/(ident+'-lod.png'))
    if not manifest.get('instances'):
        (out/'view-conditions.json').write_text(json.dumps({'method':'orthographic host geometry only; no scene instances in source gallery','prototype_angles':angles},indent=2)+'\n')
        print(out)
        return
    vertices=[];colors=[];faces=[];offset=0
    for instance in manifest['instances']:
        v,c,f=by_id[instance['prototype']];yaw=instance['yaw'];degrees=int(yaw[3:]) if isinstance(yaw,str) else int(yaw)
        a=np.deg2rad(degrees);rotation=np.round(np.array([[np.cos(a),0,np.sin(a)],[0,1,0],[-np.sin(a),0,np.cos(a)]]))
        vertices.append(v@rotation.T+instance['translation_m']);colors.append(c);faces.append(f.astype(np.int64)+offset);offset+=len(v)
    v=np.concatenate(vertices);c=np.concatenate(colors);f=np.concatenate(faces)
    canvas=Image.new('RGB',(1280,670),(25,31,39));draw=ImageDraw.Draw(canvas)
    for i,(name,angle) in enumerate([('Dense tile / above',(0.6,1,0.8)),('Dense tile / low view',(0.4,0.3,1))]):
        canvas.paste(view(v,c,f,angle,size=640),(i*640,30));draw.text((i*640+12,10),name+' (HOST geometry)',fill='white')
    canvas.save(out/'dense-tile.png')
    (out/'view-conditions.json').write_text(json.dumps({'method':'orthographic, two-sided simple host lighting; not native shader/GPU/temporal evidence','prototype_angles':angles,'lod_projection':'one bounding union across all LODs per prototype, same camera direction','scene_triangles':len(f)},indent=2)+'\n')
    print(out)

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('gallery',type=Path);args=parser.parse_args();render(args.gallery)
