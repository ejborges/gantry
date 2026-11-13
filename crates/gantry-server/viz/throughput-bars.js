// Created and resolved work per week. Library: Chart.js (global `Chart`).
const t = await gantry.api.get('/api/v1/analytics/timeline', { project: gantry.params.project, container: gantry.params.container, interval: 'week', periods: gantry.params.weeks || 12 });
const canvas = gantry.root.appendChild(document.createElement('canvas'));
canvas.setAttribute('role', 'img');
canvas.setAttribute('aria-label', 'Created and resolved work items per week');
const chart = new Chart(canvas, {
  type: 'bar',
  data: {
    labels: t.periods.map(p => gantry.formatDate(p.start, { month: 'short', day: 'numeric' })),
    datasets: [
      { label: 'Created', data: t.periods.map(p => p.created), backgroundColor: gantry.colors.series[0], borderRadius: { topLeft: 4, topRight: 4 }, maxBarThickness: 24 },
      { label: 'Resolved', data: t.periods.map(p => p.resolved), backgroundColor: gantry.colors.done, borderRadius: { topLeft: 4, topRight: 4 }, maxBarThickness: 24 },
    ],
  },
  options: {
    maintainAspectRatio: false, interaction: { mode: 'index', intersect: false },
    scales: {
      x: { ticks: { color: gantry.colors.muted }, grid: { display: false } },
      y: { beginAtZero: true, ticks: { color: gantry.colors.muted, precision: 0 }, grid: { color: gantry.colors.grid } },
    },
    plugins: { legend: { position: 'bottom', labels: { color: gantry.colors.text, usePointStyle: true } } },
  },
});
gantry.cleanup(() => chart.destroy());
