#!/usr/bin/env python3
# Public caller/service boundary check. Run with trusted daemon and harness binaries.
import hashlib,io,json,os,pathlib,subprocess,sys,tarfile,tempfile,time
base=pathlib.Path(tempfile.mkdtemp(prefix='artifactd-pub-',dir='/tmp'))
for name in ['store','runtime','publication']:
 (base/name).mkdir(mode=0o700)
def encode(value): return json.dumps(value,separators=(',',':')).encode()
def digest(data): return 'sha256:'+hashlib.sha256(data).hexdigest()
config=encode({'architecture':'amd64','os':'linux','rootfs':{'type':'layers','diff_ids':[]}})
config_desc={'mediaType':'application/vnd.oci.image.config.v1+json','digest':digest(config),'size':len(config)}
manifest=encode({'schemaVersion':2,'mediaType':'application/vnd.oci.image.manifest.v1+json','config':config_desc,'layers':[]})
index=encode({'schemaVersion':2,'mediaType':'application/vnd.oci.image.index.v1+json','manifests':[{'mediaType':'application/vnd.oci.image.manifest.v1+json','digest':digest(manifest),'size':len(manifest),'platform':{'os':'linux','architecture':'amd64'}}]})
archive=base/'publication/image.tar'
with tarfile.open(archive,'w') as tar:
 for name,data in [('oci-layout',encode({'imageLayoutVersion':'1.0.0'})),('index.json',index),('blobs/sha256/'+digest(config)[7:],config),('blobs/sha256/'+digest(manifest)[7:],manifest)]:
  info=tarfile.TarInfo(name);info.size=len(data);info.mode=0o600;tar.addfile(info,io.BytesIO(data))
archive.chmod(0o600)
daemon=pathlib.Path(sys.argv[1]).resolve()
socket=base/'runtime/artifactd.sock'
process=subprocess.Popen([str(daemon),'--store',str(base/'store'),'--socket',str(socket)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
try:
 for _ in range(500):
  if socket.exists(): break
  if process.poll() is not None: raise RuntimeError(process.stderr.read().decode())
  time.sleep(.01)
 args=[str(pathlib.Path(sys.argv[2]).resolve()),str(socket),str(os.geteuid()),str(archive),str(base/'publication/receipt.json')]
 def publish():
  p=subprocess.run(args,capture_output=True,text=True)
  assert p.returncode==0,p.stderr
  return json.loads(p.stdout)
 first=publish()
 assert first['manifest_digest']==digest(manifest)
 assert first['manifest_size']==len(manifest)
 assert first['config']=={'digest':digest(config),'media_type':config_desc['mediaType'],'size':len(config)}
 assert first['layers']==[]
 assert publish()==first
 # Remove the durable pin through the actual service API. A historical import
 # replay cannot restore it; receipt reuse must perform a new owned PIN effect.
 intent_path=base/'publication/receipt.json.artifactd-operation.json'
 intent=json.loads(intent_path.read_text())
 ctl=daemon.with_name('apollo-artifactctl')
 def action(value):
  result=subprocess.run([str(ctl),'--socket',str(socket),'--action',json.dumps(value)],capture_output=True,text=True)
  assert result.returncode==0,result.stderr
  return json.loads(result.stdout)
 action({'operation':'UNPIN','id':intent['pin']})
 assert publish()==first
 action({'operation':'GC','max_entries':4096})
 assert publish()==first
 # Alter both protected caller records consistently, recomputing receipt's
 # caller checksum. The independently verified manifest still must reject it.
 receipt_path=base/'publication/receipt.json'
 original_intent=intent_path.read_bytes(); original_receipt=receipt_path.read_bytes()
 changed=json.loads(original_receipt)
 changed['config']['size']+=1
 # receipt_payload's canonical layout is the receipt with its checksum omitted.
 changed.pop('receipt_digest')
 changed['receipt_digest']=digest(encode(changed))
 altered_intent=json.loads(original_intent);altered_intent['completion']['receipt']=changed
 intent_path.write_text(json.dumps(altered_intent));receipt_path.write_text(json.dumps(changed))
 rejected=subprocess.run(args,capture_output=True,text=True)
 assert rejected.returncode!=0 and 'ReceiptConflict' in rejected.stderr,('descriptor substitution did not fail at manifest binding',rejected.stderr)
 intent_path.write_bytes(original_intent);receipt_path.write_bytes(original_receipt)
 assert publish()==first
 archive.unlink()
 assert publish()==first
 print('PASS direct FD import with independently hashed OCI manifest/config; repeat; pin restoration survives GC; descriptor substitution rejected; receipt reuse after archive removal')
 print('qualification fixture:',base)
finally:
 process.kill();process.wait()
