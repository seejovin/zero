#!/usr/bin/env bash
set -euo pipefail

wallet_root="$(cd "$(dirname "$0")/.." && pwd)"
wallet_sdk="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
wallet_ndk="${ANDROID_NDK_HOME:-${wallet_sdk}/ndk/27.2.12479018}"
wallet_gradle="${WALLET_GRADLE:-${wallet_root}/android/gradlew}"
wallet_cargo="${WALLET_CARGO:-cargo}"
wallet_rustup="${WALLET_RUSTUP:-rustup}"
if [[ -z "$wallet_sdk" || ! -d "$wallet_ndk" ]]; then
    printf '%s\n' 'Set ANDROID_SDK_ROOT and install platform 35, build-tools 35.0.0, and NDK 27.2.12479018.' >&2
    exit 1
fi
case "$(uname -s)" in
    Linux) wallet_ndk_host="linux-x86_64" ;;
    Darwin) wallet_ndk_host="darwin-x86_64" ;;
    *) printf '%s\n' 'Build on Linux or macOS with the Android NDK.' >&2; exit 1 ;;
esac
wallet_ndk_bin="${wallet_ndk}/toolchains/llvm/prebuilt/${wallet_ndk_host}/bin"
for wallet_required_tool in aarch64-linux-android30-clang x86_64-linux-android30-clang llvm-ar; do
    if [[ ! -x "${wallet_ndk_bin}/${wallet_required_tool}" ]]; then
        printf 'Missing NDK tool: %s\n' "$wallet_required_tool" >&2
        exit 1
    fi
done
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${wallet_ndk_bin}/aarch64-linux-android30-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="${wallet_ndk_bin}/llvm-ar"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="${wallet_ndk_bin}/x86_64-linux-android30-clang"
export CC_x86_64_linux_android="$CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER"
export AR_x86_64_linux_android="${wallet_ndk_bin}/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
export CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
cd "$wallet_root"
"$wallet_rustup" target add aarch64-linux-android x86_64-linux-android
"$wallet_cargo" build --locked --release -p wallet_core
case "$(uname -s)" in
    Darwin) wallet_host_lib="$wallet_root/target/release/libwallet_core.dylib" ;;
    *) wallet_host_lib="$wallet_root/target/release/libwallet_core.so" ;;
esac
mkdir -p android/app/src/main/generated android/app/src/main/jniLibs/arm64-v8a android/app/src/main/jniLibs/x86_64
"$wallet_cargo" run --locked --release -p wallet_core --features bindgen --bin uniffi-bindgen -- generate \
    --library "$wallet_host_lib" --language kotlin --config core/uniffi.toml --out-dir android/app/src/main/generated
"$wallet_cargo" build --locked --release -p wallet_core --target aarch64-linux-android
"$wallet_cargo" build --locked --release -p wallet_core --target x86_64-linux-android
cp target/aarch64-linux-android/release/libwallet_core.so android/app/src/main/jniLibs/arm64-v8a/
cp target/x86_64-linux-android/release/libwallet_core.so android/app/src/main/jniLibs/x86_64/
cd android
"$wallet_gradle" --no-daemon --console=plain :app:testReleaseUnitTest :app:lintRelease :app:assembleRelease "$@"
printf '%s\n' "Unsigned release APK: $wallet_root/android/app/build/outputs/apk/release/app-release-unsigned.apk"
printf '%s\n' 'Align with zipalign -P 16, then sign with your own app distribution key using apksigner. Wallet keys are separate.'
