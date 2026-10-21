# shellcheck shell=bash
# Decide which URL proves a site-server deploy.
#
# 200 from /api/ready is the current check. 404 means this image was built
# before that route existed (a rollback, or CI has not published the new
# image yet). In that case the deploy check uses /api/health. Any other
# status, including connection failure, keeps waiting. A 503 is a live
# readiness failure and must not be treated as an old image.

# Prints ready, health, or wait.
deploy_probe_choice() {
    case "$1" in
        200) printf 'ready\n' ;;
        404) printf 'health\n' ;;
        *) printf 'wait\n' ;;
    esac
}
