(async () => {
  const vp = document.querySelector('.viewport');
  const t = [];
  let last = performance.now();
  let y = 0;
  const end = last + 4000;
  await new Promise((res) => {
    function f(now) {
      t.push(now - last);
      last = now;
      y = (y + 90) % 40000;
      vp.scrollTop = y;
      if (now < end) requestAnimationFrame(f);
      else res();
    }
    requestAnimationFrame(f);
  });
  t.shift();
  const s = [...t].sort((a, b) => a - b);
  const pick = (q) => +s[Math.min(s.length - 1, Math.floor(s.length * q))].toFixed(1);
  const dropped = t.filter((x) => x > 25).length;
  return { frames: t.length, fps: +(t.length / 4).toFixed(1), p50ms: pick(0.5), p95ms: pick(0.95), p99ms: pick(0.99), maxMs: +s[s.length - 1].toFixed(1), framesOver25ms: dropped };
})()
