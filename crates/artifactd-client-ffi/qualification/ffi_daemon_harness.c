#include "artifactd_client.h"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static void die(ArtifactdClient *c, const char *where) { char e[512]={0}; artifactd_last_error(e,sizeof e); fprintf(stderr,"%s: %s\n",where,e); artifactd_client_free(c); exit(1); }
static void op(ArtifactdClient *c, char out[128]) { if (artifactd_operation_allocate(c,out,128)!=0) die(c,"allocate"); }
static ArtifactdFacts *call_lease(ArtifactdClient *c,const char *d,const char *o) { ArtifactdFacts *f=NULL; if(artifactd_lease_create(c,o,d,getuid(),getgid(),&f)!=0) die(c,"lease_create"); return f; }
int main(int argc,char **argv) {
 if(argc!=3){fprintf(stderr,"usage: %s SOCKET INDEX_DIGEST\n",argv[0]);return 2;}
 ArtifactdClient *c=NULL; if(artifactd_client_new(argv[1],(unsigned)getuid(),&c)!=0) die(NULL,"client_new");
 char o[128], digest[256]; strncpy(digest,argv[2],sizeof(digest));
 ArtifactdFacts *f=NULL; op(c,o); if(artifactd_resolve(c,o,digest,"linux","amd64",NULL,&f)!=0) die(c,"resolve");
 char json[65536]; if(artifactd_facts_json(f,json,sizeof json)!=0) die(c,"resolve_json"); printf("resolve=%s\n",json); artifactd_facts_free(f); f=NULL;
 op(c,o); if(artifactd_pin(c,o,"ffi-pin",digest,&f)!=0) die(c,"pin"); artifactd_facts_free(f); f=NULL;
 op(c,o); f=call_lease(c,digest,o); char lease[256]; if(artifactd_facts_get(f,"lease_id",lease,sizeof lease)!=0) die(c,"lease_id"); printf("lease=%s\n",lease); artifactd_facts_free(f);
 op(c,o); int fd=-1; if(artifactd_open_blob(c,o,digest,lease,&fd)!=0) die(c,"open_blob"); char b[16]={0}; ssize_t n=read(fd,b,sizeof b); if(n<=0){perror("read");return 1;} printf("open_blob_bytes=%zd\n",n); artifactd_fd_close(fd);
 op(c,o); if(artifactd_lease_release(c,o,lease,&f)!=0) die(c,"lease_release"); artifactd_facts_free(f); f=NULL;
 op(c,o); if(artifactd_unpin(c,o,"ffi-pin",&f)!=0) die(c,"unpin"); artifactd_facts_free(f); artifactd_client_free(c); puts("FFI_HARNESS_OK"); return 0;
}
