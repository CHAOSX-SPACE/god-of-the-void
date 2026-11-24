# Parche de GOD OF THE VOID sobre mlx-sys 0.7.0

Única diferencia con el crate publicado: `src/mlx-c/CMakeLists.txt` compila MLX **v0.32.4** en vez de v0.32.2.

Medido 2026-11-10 (M11): la `quantized_matmul` de MLX 0.32.2 se aleja del resultado f32 correcto (22 670 de
55 296 valores iguales al f32 bien redondeado en `q_proj` de Qwen3-Embedding 4-bit; error medio 7,5·10⁻⁴); la de
0.32.4 acierta 55 289 de 55 296 (error medio 3,0·10⁻⁴). Todos los modelos cuantizados del núcleo ganan precisión.
La rFFT en GPU también cambió entre ambas. Quitar este parche cuando mlx-sys publique una versión con MLX ≥ 0.32.4.

Segundo cambio, obligado por el primero: MLX 0.32.4 insertó `global_scale` en `mlx::core::gather_qmm` (antes de
`sorted_indices`); `src/mlx-c/mlx/c/ops.cpp` (`mlx_gather_qmm`) le pasa `std::nullopt`. Sin él, mlx-c no compila.
