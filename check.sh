#!/bin/sh

# This is a simple script to run all the necessary checks locally.

set -e

echo "Rustfmt"
cargo fmt --all

for_all_features() {
	echo
	echo "features: none"
	echo "--------------"
	$* --no-default-features

	echo
	echo "features: macro"
	echo "---------------"
	$* --no-default-features --features macro

	echo
	echo "features: alloc"
	echo "---------------"
	$* --no-default-features --features alloc

	echo
	echo "features: alloc, macro"
	echo "----------------------"
	$* --no-default-features --features alloc,macro

	MIRIFLAGS="$MIRIFLAGS -Zmiri-disable-isolation"

	echo
	echo "features: std"
	echo "-------------"
	$* --no-default-features --features std

	echo
	echo "features: std, macro"
	echo "--------------------"
	$* --no-default-features --features std,macro

	unset MIRIFLAGS
}

echo
echo "Clippy"
echo "======"
for_all_features cargo clippy --workspace --all-targets

echo
echo "completion-io: cross platform clippy"
echo "===================================="
cargo clippy -p completion-io --all-targets --target x86_64-unknown-linux-gnu
cargo clippy -p completion-io --all-targets --target x86_64-pc-windows-gnu
cargo clippy -p completion-io --all-targets --target wasm32-wasi
cargo clippy -p completion-io --all-targets --target wasm32-unknown-unknown

echo
echo "Doctests"
echo "========"
cargo test --workspace --doc

echo
echo "Tests"
echo "====="
for_all_features cargo test --lib --tests --workspace

echo
echo "Miri tests"
echo "=========="
MIRIFLAGS="-Zmiri-track-raw-pointers" for_all_features cargo +nightly miri test --lib --tests --workspace

echo
echo "Miri doc tests"
echo "=============="
MIRIFLAGS="-Zmiri-track-raw-pointers" cargo +nightly miri test --workspace --doc

echo
echo "Rustdoc"
echo "======="
cargo doc --no-deps --workspace

echo
echo "Spell check"
echo "==========="
cargo spellcheck

echo
echo "Success!"
