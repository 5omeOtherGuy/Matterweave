"""Host-only orthographic geometry inspection, not engine shading/performance evidence."""
import argparse, json
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw

def read_obj(path):
    vertices, colors, faces = [], [], []
    for line in Path(path).read_text().splitlines():
        p = line.split()
        if not p: continue
        if p[0] == 'v':
            vertices.append([float(v) for v in p[1:4]])
            colors.append([float(v) for v in p[4:7]] if len(p)>=7 else [0.75,0.60,0.40])
        elif p[0] == 'f':
            ids = [int(v.split('/')[0]) for v in p[1:]]
            ids = [v-1 if v>0 else len(vertices)+v for v in ids]
            for i in range(1,len(ids)-1): faces.append([ids[0],ids[i],ids[i+1]])
    return np.array(vertices),np.array(colors),np.array(faces)

def view(v,c,faces,direction,size=480,silhouette=False,reference=None):
    forward=np.array(direction,dtype=float);forward/=np.linalg.norm(forward)
    up=np.array([0.,1.,0.])
    right=np.cross(up,forward);right/=np.linalg.norm(right)
    up=np.cross(forward,right)
    reference=v if reference is None else reference
    center=(reference.max(axis=0)+reference.min(axis=0))/2
    basis=np.array([right,up,forward]).T
    q=(v-center)@basis
    ref_q=(reference-center)@basis
    span=max(np.ptp(ref_q[:,0]),np.ptp(ref_q[:,1]))*1.15
    q[:,0]=q[:,0]/span*(size-1)+(size-1)/2
    q[:,1]=-q[:,1]/span*(size-1)+(size-1)/2
    rgb=np.full((size,size,3),[25,31,39],dtype=np.uint8)
    depth=np.full((size,size),-np.inf)
    light=np.array([0.4,0.8,0.5]);light/=np.linalg.norm(light)
    for face in faces:
        p=q[face];a,b,d=p[:,:2]
        denom=(b[1]-d[1])*(a[0]-d[0])+(d[0]-b[0])*(a[1]-d[1])
        if abs(denom)<1e-9:continue
        lo=np.maximum(np.floor(p[:,:2].min(axis=0)).astype(int),0)
        hi=np.minimum(np.ceil(p[:,:2].max(axis=0)).astype(int),size-1)
        if np.any(hi<lo):continue
        x,y=np.meshgrid(np.arange(lo[0],hi[0]+1)+0.5,np.arange(lo[1],hi[1]+1)+0.5)
        u=((b[1]-d[1])*(x-d[0])+(d[0]-b[0])*(y-d[1]))/denom
        w=((d[1]-a[1])*(x-d[0])+(a[0]-d[0])*(y-d[1]))/denom
        t=1-u-w;z=u*p[0,2]+w*p[1,2]+t*p[2,2]
        region=depth[lo[1]:hi[1]+1,lo[0]:hi[0]+1]
        mask=(u>=-1e-7)&(w>=-1e-7)&(t>=-1e-7)&(z>region)
        normal=np.cross(v[face[1]]-v[face[0]],v[face[2]]-v[face[0]])
        n=np.linalg.norm(normal)
        shade=0.45+0.55*abs(float(np.dot(normal/n,light))) if n else 0.45
        color=np.array([220,223,226]) if silhouette else np.clip(c[face].mean(axis=0)*shade*255,0,255).astype(np.uint8)
        region[mask]=z[mask]
        rgb[lo[1]:hi[1]+1,lo[0]:hi[0]+1][mask]=color
    return Image.fromarray(rgb)

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('obj');p.add_argument('output');args=p.parse_args()
    v,c,f=read_obj(args.obj)
    views=[('Front',(0,0.15,1),False),('Side',(1,0.15,0),False),('Underside',(0.4,-1,0.6),False),('Above',(0.6,1,0.8),False),('Silhouette',(0,0,1),True),('Side silhouette',(1,0,0),True)]
    canvas=Image.new('RGB',(1440,1020),(25,31,39));draw=ImageDraw.Draw(canvas)
    for i,(name,direction,silhouette) in enumerate(views):
        x=(i%3)*480;y=(i//3)*510
        canvas.paste(view(v,c,f,direction,silhouette=silhouette),(x,y+30));draw.text((x+12,y+10),name,fill='white')
    canvas.save(args.output)
    Path(args.output+'.json').write_text(json.dumps({'source':args.obj,'method':'orthographic host two-sided geometry inspection, no engine shadow/material qualification','bounds':[v.min(axis=0).tolist(),v.max(axis=0).tolist()],'vertices':len(v),'triangles':len(f),'views':views},indent=2)+'\n')
