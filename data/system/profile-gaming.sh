# SegLinux: room for the shader caches of many games.
#
# Mesa's disk cache (~/.cache/mesa_shader_cache): DXVK 2 and VKD3D-Proton keep
# their compiled shaders there; with Mesa's 1 GB default, installing several
# games evicts them and games stutter while recompiling.
# NVIDIA's driver has its own cache (~/.nv/GLCache, the compute cache and the
# shader cache of Vulkan); its size is in bytes, and by default the driver
# deletes the cache once it is full, so the cleanup is switched off.
# Set your own values to override: they are only set here when not set yet.
# (/etc/environment.d is for systemd; this works on every init.)
: "${MESA_SHADER_CACHE_MAX_SIZE:=10G}"
: "${__GL_SHADER_DISK_CACHE_SIZE:=10737418240}"
: "${__GL_SHADER_DISK_CACHE_SKIP_CLEANUP:=1}"
export MESA_SHADER_CACHE_MAX_SIZE __GL_SHADER_DISK_CACHE_SIZE __GL_SHADER_DISK_CACHE_SKIP_CLEANUP
