// Open work over time: how much was unfinished at the end of each week. Library: Chart.js.
const t = await gantry.api.get('/api/v1/analytics/timeline', { project: gantry.params.project, container: gantry.params.container, interval: 'week', periods: gantry.params.weeks || 16 });
const canvas = gantry.root.appendChild(document.createElement('canvas'));
canvas.setAttribute('role', 'img');
canvas.setAttribute('aria-label', 'Open work items at the end of each week');
const chart = new Chart(canvas, {
  type: 'line',
  data: {
    labels: t.periods.map(p => gantry.formatDate(p.start, { month: 'short', day: 'numeric' })),
    datasets: [{ label: 'Open at end of week', data: t.periods.map(p => p.open_at_end), borderColor: gantry.colors.accent, backgroundColor: gantry.colors.accentSoft, borderWidth: 2, fill: true, tension: 0.25, pointRadius: 0, pointHoverRadius: 5, pointHitRadius: 16 }],
  },
  options: {
    maintainAspectRatio: false, interaction: { mode: 'index', intersect: false },
    scales: {
      x: { ticks: { color: gantry.colors.muted }, grid: { display: false } },
      y: { beginAtZero: true, ticks: { color: gantry.colors.muted, precision: 0 }, grid: { color: gantry.colors.grid } },
    },
    plugins: { legend: { display: false } },
  },
});
gantry.cleanup(() => chart.destroy());
