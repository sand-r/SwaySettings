/*
 * Link-only PAM shim for building inside the Codex Flatpak SDK.
 *
 * The SDK has no PAM development package. This shared object deliberately has
 * the same SONAME as PAM and exports only the functions the locker references,
 * allowing the linker to record a libpam.so.0 dependency. It must never be
 * bundled or placed on the runtime library path: the installed locker resolves
 * libpam.so.0 to the host's real PAM implementation.
 */

typedef struct pam_handle pam_handle_t;
typedef struct pam_conv pam_conv_t;

int pam_start(const char *service_name, const char *user, const pam_conv_t *conv,
              pam_handle_t **pamh) {
    (void)service_name;
    (void)user;
    (void)conv;
    (void)pamh;
    return 4; /* PAM_SYSTEM_ERR */
}

int pam_authenticate(pam_handle_t *pamh, int flags) {
    (void)pamh;
    (void)flags;
    return 4;
}

int pam_setcred(pam_handle_t *pamh, int flags) {
    (void)pamh;
    (void)flags;
    return 4;
}

int pam_end(pam_handle_t *pamh, int pam_status) {
    (void)pamh;
    (void)pam_status;
    return 4;
}
