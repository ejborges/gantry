// Work by status category, as a doughnut. Library: Chart.js (global `Chart`).
const data = await gantry.api.get('/api/v1/analytics/items', { project: gantry.params.project, container: gantry.params.container, group_by: 'status_category' });
const order = ['todo', 'in_progress', 'done'];
const groups = order.map(k => data.groups.find(g => g.key === k) || { key: k, label: k, value: 0 });
const canvas = gantry.root.appendChild(document.createElement('canvas'));
canvas.setAttribute('role', 'img');
canvas.setAttribute('aria-label', 'Work by status: ' + groups.map(g => g.value + ' ' + g.label).join(', '));
const chart = new Chart(canvas, {
  type: 'doughnut',
  data: {
    labels: ['To do', 'In progress', 'Done'].map((name, i) => name + ' · ' + gantry.formatNumber(groups[i].value)),
    datasets: [{ data: groups.map(g => g.value), backgroundColor: [gantry.colors.todo, gantry.colors.progress, gantry.colors.done], borderColor: gantry.colors.background, borderWidth: 2 }],
  },
  options: {
    maintainAspectRatio: false, cutout: '62%',
    plugins: { legend: { position: 'bottom', labels: { color: gantry.colors.text, usePointStyle: true } } },
  },
});
gantry.cleanup(() => chart.destroy());
