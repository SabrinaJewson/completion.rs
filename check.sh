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
echo "Doctests"
echo "========"
cargo test --workspace --doc

echo
echo "Tests"
echo "====="
for_all_features cargo test --lib --tests --workspace

echo
echo "Miri"
echo "===="
MIRIFLAGS="-Zmiri-track-raw-pointers" for_all_features cargo +nightly miri test

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
