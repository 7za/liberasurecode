#! /usr/bin/env bash

set -veux

BUILD_DIR=$PWD
MAKE_FLAGS=""
export CPATH="${BUILD_DIR}/include:${BUILD_DIR}/include/jerasure:${CPATH:-}"
export LIBRARY_PATH="${BUILD_DIR}/lib:${LIBRARY_PATH:-}"

# Please try and add other distributions.
case "$(uname)" in
    "Linux") MAKE_FLAGS="-j$(nproc)";;
    "Darwin") MAKE_FLAGS="-j$(sysctl -n hw.ncpu)"
esac

export CFLAGS="-O2"

#
# gf-complete
#
git clone https://github.com/ceph/gf-complete.git
cd gf-complete/
git checkout a6862d1
./autogen.sh
if [[ $DEBUG = true ]]; then
    ./configure --disable-shared --with-pic --prefix $BUILD_DIR CFLAGS="${CFLAGS:-}"
else
    ./configure --disable-shared --with-pic --prefix $BUILD_DIR CFLAGS="${CFLAGS:-}"
fi
make $MAKE_FLAGS install
cd ../

#
# jerasure
#
git clone https://github.com/ceph/jerasure.git
cd jerasure/
git checkout de1739c
autoreconf --force --install
if [[ $DEBUG = true ]]; then
    ./configure --disable-shared --enable-static --with-pic --prefix $BUILD_DIR CFLAGS="${CFLAGS:-}"
else
    ./configure --disable-shared --enable-static --with-pic --prefix $BUILD_DIR
fi
make $MAKE_FLAGS install
cd ../

#
# liberasurecode
#
git clone https://github.com/scality/liberasurecode
cd liberasurecode/
echo $PWD
git checkout development/1.0
patch -p1 < ../for_darwin_to_detect_compiler_flag.patch
./autogen.sh
if [[ $DEBUG = true ]]; then
    LIBS="-lJerasure" ./configure --disable-shared --with-pic --prefix $BUILD_DIR CFLAGS="${CFLAGS:-}"
else
    LIBS="-lJerasure" ./configure --disable-shared --with-pic --prefix $BUILD_DIR CFLAGS="$CFLAGS"
fi
#patch -p1 < ../liberasurecode.patch # Applies a patch for building static library
#patch -p1 < ../liberasurecode.patch # Applies a patch for building static library
make $MAKE_FLAGS install
