# SegLinux: room for the shaders of many games in Mesa's disk cache
# (~/.cache/mesa_shader_cache). DXVK 2 and VKD3D-Proton keep their compiled
# shaders there; with Mesa's 1 GB default, installing several games evicts
# them and games stutter while recompiling. Set your own value to override.
: "${MESA_SHADER_CACHE_MAX_SIZE:=4G}"
export MESA_SHADER_CACHE_MAX_SIZE
