#!/bin/bash
# Experimento de precisão energética (AGENTS §1.4) — laboratório 31/08/2026.
# Compara a energia atribuída pelo Caderno (partilha da potência RAPL real,
# Σ W×1s virtual) com a energia medida pelo RAPL (Δenergy_uj) na MESMA janela.
# Referência full-to-full: o runtime atribui a potência integral do package.
set -u
cd /home/silvano-neto/Documents/verbo
V=./core/target/release/vbl
RAPL=/sys/devices/virtual/powercap/intel-rapl/intel-rapl:0/energy_uj
CFG=logs/stage5/lab/fxp-lab.cfg
VL=logs/stage5/lab/lab-energy.vl
E() { cat "$RAPL"; }
J() { grep -oE "[0-9.]+ J acumulados" | grep -oE "^[0-9.]+"; }

echo "# janela de repouso (60 s) — fundo: soak 24h + desktop"
e0=$(E); s0=$(date +%s.%N); sleep 60; e1=$(E); s1=$(date +%s.%N)
python3 -c "print(f'repouso: {($e1-$e0)/1e6/($s1-$s0):.2f} W média')"

for i in 1 2 3; do
  echo "# carga $i (92 ticks × 1 s de parede)"
  e0=$(E)
  OUT=$($V run "$VL" --ticks 92 --real-ms 1000 --fxp-config "$CFG" --caderno "logs/stage5/lab/rapl-precision-$i.vcad" 2>&1)
  e1=$(E)
  ECAD=$(echo "$OUT" | J)
  echo "$OUT" | grep -E "ÍNTEGRA"
  echo "RESULTADO $i: ecad=$ECAD erapl_uj=$(($e1-$e0)) parede=$(echo "$OUT" | grep -oE '[0-9]+ tick\(s\) em [0-9.]+m?s' | head -1)"
done
echo "# fim"
