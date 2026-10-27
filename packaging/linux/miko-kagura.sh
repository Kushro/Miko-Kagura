#!/bin/sh
# /usr/bin/miko-kagura for the .deb and AUR packages; the app lives in /opt/miko-kagura.
# WebKitGTK's DMA-BUF renderer shows a blank window on some NVIDIA/Wayland setups;
# export WEBKIT_DISABLE_DMABUF_RENDERER=0 to opt back in.
: "${WEBKIT_DISABLE_DMABUF_RENDERER:=1}"
export WEBKIT_DISABLE_DMABUF_RENDERER
exec /opt/miko-kagura/miko-kagura "$@"
