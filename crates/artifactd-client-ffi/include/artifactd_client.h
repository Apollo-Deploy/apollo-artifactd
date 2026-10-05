#ifndef APOLLO_ARTIFACTD_CLIENT_H
#define APOLLO_ARTIFACTD_CLIENT_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct ArtifactdClient ArtifactdClient;
typedef struct ArtifactdFacts ArtifactdFacts;
int32_t artifactd_client_new(const char *socket_path, uint32_t server_uid, ArtifactdClient **out);
void artifactd_client_free(ArtifactdClient *client);
int32_t artifactd_last_error(char *out, uintptr_t capacity);
int32_t artifactd_operation_allocate(ArtifactdClient *, char *out, uintptr_t capacity);
int32_t artifactd_pull(ArtifactdClient *, const char *operation, const char *reference, const char *os, const char *architecture, const char *variant, const char *pin, ArtifactdFacts **out);
int32_t artifactd_pull_with_credentials(ArtifactdClient *, const char *operation, const char *reference, const char *os, const char *architecture, const char *variant, const char *pin, int32_t credentials_fd, ArtifactdFacts **out);
int32_t artifactd_resolve(ArtifactdClient *, const char *operation, const char *digest, const char *os, const char *architecture, const char *variant, ArtifactdFacts **out);
int32_t artifactd_verify(ArtifactdClient *, const char *operation, const char *digest, ArtifactdFacts **out);
int32_t artifactd_pin(ArtifactdClient *, const char *operation, const char *pin, const char *digest, ArtifactdFacts **out);
int32_t artifactd_unpin(ArtifactdClient *, const char *operation, const char *pin, ArtifactdFacts **out);
int32_t artifactd_lease_create(ArtifactdClient *, const char *operation, const char *digest, uint32_t grantee_uid, uint32_t grantee_gid, ArtifactdFacts **out);
int32_t artifactd_lease_release(ArtifactdClient *, const char *operation, const char *lease, ArtifactdFacts **out);
int32_t artifactd_prepare(ArtifactdClient *, const char *operation, const char *digest, const char *os, const char *architecture, const char *variant, ArtifactdFacts **out);
int32_t artifactd_open_blob(ArtifactdClient *, const char *operation, const char *digest, const char *lease, int32_t *out_fd);
int32_t artifactd_open_prepared(ArtifactdClient *, const char *operation, const char *prepared, const char *lease, int32_t *out_fd);
int32_t artifactd_runtime_config(ArtifactdClient *, const char *operation, const char *config_digest, const char *lease, ArtifactdFacts **out);
void artifactd_facts_free(ArtifactdFacts *facts);
int32_t artifactd_facts_json(const ArtifactdFacts *, char *out, uintptr_t capacity);
int32_t artifactd_facts_get(const ArtifactdFacts *, const char *key, char *out, uintptr_t capacity);
int32_t artifactd_facts_artifact_digest(const ArtifactdFacts *, char *out, uintptr_t capacity);
int32_t artifactd_facts_manifest_digest(const ArtifactdFacts *, char *out, uintptr_t capacity);
int32_t artifactd_facts_config_digest(const ArtifactdFacts *, char *out, uintptr_t capacity);
int32_t artifactd_facts_manifest_size(const ArtifactdFacts *, uint64_t *out);
int32_t artifactd_facts_config_size(const ArtifactdFacts *, uint64_t *out);
void artifactd_fd_close(int32_t fd);
#ifdef __cplusplus
}
#endif
#endif
