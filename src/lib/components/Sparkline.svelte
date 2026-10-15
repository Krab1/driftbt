<script lang="ts">
  /** Two small lines (download, upload) sharing one scale. */
  let { down, up, width = 96, height = 18, label }: { down: number[]; up: number[]; width?: number; height?: number; label: string } = $props();

  const max = $derived(Math.max(1, ...down, ...up));
  function points(v: number[]): string {
    if (v.length < 2) return "";
    const step = width / (v.length - 1);
    return v.map((x, i) => `${(i * step).toFixed(1)},${(height - 1 - (x / max) * (height - 2)).toFixed(1)}`).join(" ");
  }
</script>

<svg {width} {height} viewBox="0 0 {width} {height}" preserveAspectRatio="none" role="img" aria-label={label}>
  <title>{label}</title>
  <polyline class="u" points={points(up)} />
  <polyline class="d" points={points(down)} />
</svg>

<style>
  svg {
    flex: none;
    overflow: visible;
  }
  polyline {
    fill: none;
    stroke-width: 1.5;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }
  .d {
    stroke: var(--accent);
  }
  .u {
    stroke: var(--success);
  }
</style>
