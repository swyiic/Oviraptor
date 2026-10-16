# Source only for the agent-first task; historical target directories are protected.
source /workspace/toolchain/accepted-test-env.sh
export CARGO_TARGET_DIR=/workspace/asset-accuracy-target
export CARGO_INCREMENTAL=0
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_TEST_OPT_LEVEL=1
export CARGO_PROFILE_TEST_DEBUG_ASSERTIONS=true
export CARGO_PROFILE_TEST_OVERFLOW_CHECKS=true
export OVIRAPTOR_TEST_NO_SANDBOX=1
export CHROME_BINARY=/usr/bin/chromium

export RUSTC_WORKSPACE_WRAPPER=/workspace/validation/agent-first-20261016/native-next-stage/phase2/rustc-workspace-isolated.sh
