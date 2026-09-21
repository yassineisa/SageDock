#!/bin/bash
# Builds the contents of the SageDock runtime image inside a fresh Ubuntu base distro.
#
# Run as root by scripts/build-runtime.ps1 — never by the SageDock app, and never on an
# end user's machine. Everything slow and network-dependent about getting SageMath
# happens here, once, with retries, so students only ever import a finished image.
#
# The image exposes a small, stable contract the app depends on, so the internals below
# (conda vs apt, versions, paths inside the environment) can change without an app update:
#   /opt/sagedock/runtime.json          format version + SageMath version
#   /opt/sagedock/bin/sagedock-env      runs a command inside the SageMath environment
#   /opt/sagedock/bin/sagedock-jupyter  starts JupyterLab (arguments passed through)
#   /opt/sagedock/bin/sagedock-verify   prints machine-checkable health results
#   /opt/sagedock/bin/sagedock-selftest executes a real cell through the Sage kernel
#   Jupyter kernels named exactly "sagemath" and "python3"
#   A non-root user "sage" that notebooks run as

set -eo pipefail

SAGE_VERSION="${1:?usage: provision.sh <sage-version> <miniforge-version>}"
MINIFORGE_VERSION="${2:?usage: provision.sh <sage-version> <miniforge-version>}"
ARCH="$(uname -m)"

export DEBIAN_FRONTEND=noninteractive
# conda's own retry knobs; the retry() wrapper below covers failures beyond these.
export CONDA_REMOTE_MAX_RETRIES=8
export CONDA_REMOTE_BACKOFF_FACTOR=3
export CONDA_REMOTE_READ_TIMEOUT_SECS=180

log() { echo "[provision] $*"; }

# Package mirrors and CDNs drop connections on multi-gigabyte downloads often enough that
# a single attempt does not make a reliable build.
retry() {
    local attempt=1 max=6
    until "$@"; do
        if [ "$attempt" -ge "$max" ]; then
            log "giving up after $attempt attempts: $*"
            return 1
        fi
        log "attempt $attempt failed; retrying in $((attempt * 15))s: $*"
        sleep $((attempt * 15))
        attempt=$((attempt + 1))
    done
}

log "system packages"
retry apt-get update
retry apt-get install -y --no-install-recommends ca-certificates curl bzip2 git locales tzdata
sed -i 's/^# *en_US.UTF-8/en_US.UTF-8/' /etc/locale.gen
locale-gen en_US.UTF-8
update-locale LANG=en_US.UTF-8

log "notebook user"
id -u sage >/dev/null 2>&1 || useradd --create-home --shell /bin/bash sage

# appendWindowsPath=false keeps Windows python.exe/jupyter.exe from shadowing the Linux
# ones, which otherwise produces failures that are very hard to diagnose from a notebook.
cat > /etc/wsl.conf <<'EOF'
[user]
default=sage

[boot]
systemd=false

[interop]
enabled=true
appendWindowsPath=false

[network]
generateResolvConf=true
EOF

log "miniforge ${MINIFORGE_VERSION} (${ARCH})"
installer=/tmp/miniforge.sh
retry curl -fL --retry 5 --retry-delay 10 -o "$installer" \
    "https://github.com/conda-forge/miniforge/releases/download/${MINIFORGE_VERSION}/Miniforge3-Linux-${ARCH}.sh"
rm -rf /opt/conda
bash "$installer" -b -p /opt/conda
rm -f "$installer"

MAMBA=/opt/conda/bin/mamba
[ -x "$MAMBA" ] || MAMBA=/opt/conda/bin/conda

# Recreated from scratch on each attempt: resuming a half-created environment can leave it
# inconsistent in ways that only show up later as import errors.
create_env() {
    rm -rf /opt/sagedock/env
    "$MAMBA" create -y -p /opt/sagedock/env -c conda-forge --override-channels \
        "sage=${SAGE_VERSION}" "python=3.12" \
        jupyterlab ipykernel nbconvert ipywidgets \
        pandas scikit-learn
}

log "SageMath ${SAGE_VERSION} and notebook stack (the large download)"
mkdir -p /opt/sagedock
retry create_env

# The app creates notebooks that request a kernel named exactly "sagemath". Normalize in
# case the packaged kernelspec carries a version suffix.
KDIR=/opt/sagedock/env/share/jupyter/kernels
if [ ! -d "$KDIR/sagemath" ]; then
    candidate="$(ls -d "$KDIR"/sagemath* 2>/dev/null | head -n1 || true)"
    if [ -n "$candidate" ]; then
        log "normalizing kernelspec $(basename "$candidate") -> sagemath"
        cp -r "$candidate" "$KDIR/sagemath"
    fi
fi

log "entry points"
mkdir -p /opt/sagedock/bin

cat > /opt/sagedock/bin/sagedock-env <<'EOF'
#!/bin/bash
# Runs a command inside SageDock's SageMath environment. Activation (not just PATH) matters:
# several of SageMath's components rely on variables set by conda activation scripts.
source /opt/conda/etc/profile.d/conda.sh
conda activate /opt/sagedock/env
exec "$@"
EOF

cat > /opt/sagedock/bin/sagedock-jupyter <<'EOF'
#!/bin/bash
exec /opt/sagedock/bin/sagedock-env jupyter lab "$@"
EOF

cat > /opt/sagedock/bin/sagedock-verify <<'EOF'
#!/bin/bash
# Machine-checkable health results, one KEY=value per line. Read by the SageDock app.
run() { /opt/sagedock/bin/sagedock-env "$@"; }
echo "SAGE_VERSION=$(run sage --version 2>&1 | head -n1)"
echo "SAGE_FACTOR=$(run sage -c 'print(factor(123456))' 2>&1 | tail -n1)"
echo "PYTHON_IMPORTS=$(run python -c 'import numpy, scipy, sympy, pandas, matplotlib, sklearn; print("ok")' 2>&1 | tail -n1)"
echo "KERNELS=$(run jupyter kernelspec list 2>&1 | tr '\n' ' ')"
EOF

cat > /opt/sagedock/bin/sagedock-selftest <<'EOF'
#!/bin/bash
# Executes a real notebook cell through the SageMath kernel: stronger evidence than the
# server merely starting, because it exercises the kernel protocol end to end.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cat > "$work/test.ipynb" <<'NB'
{"cells": [{"cell_type": "code", "execution_count": null, "metadata": {}, "outputs": [],
            "source": ["print(factor(123456))"]}],
 "metadata": {"kernelspec": {"display_name": "SageMath", "language": "sage", "name": "sagemath"}},
 "nbformat": 4, "nbformat_minor": 5}
NB
if /opt/sagedock/bin/sagedock-env jupyter nbconvert --to notebook --execute "$work/test.ipynb" \
        --output out --output-dir "$work" --ExecutePreprocessor.timeout=600 >"$work/log.txt" 2>&1 \
    && grep -qF '2^6 * 3 * 643' "$work/out.ipynb"; then
    echo "KERNEL_OK"
else
    echo "KERNEL_FAIL"
    tail -n 40 "$work/log.txt"
fi
EOF

chmod 755 /opt/sagedock/bin/*

cat > /opt/sagedock/runtime.json <<EOF
{
  "format": 1,
  "sage_version": "${SAGE_VERSION}",
  "miniforge_version": "${MINIFORGE_VERSION}",
  "arch": "${ARCH}",
  "built_utc": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

log "shrinking image"
"$MAMBA" clean -afy || true
apt-get clean
rm -rf /var/lib/apt/lists/* /tmp/* /root/.cache

echo "PROVISION_OK"
