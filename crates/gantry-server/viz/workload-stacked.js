// Who has what: open work per assignee, split by status. Library: Apache ECharts.
const w = await gantry.api.get('/api/v1/analytics/workload', { project: gantry.params.project, container: gantry.params.container });
const people = w.people.slice(0, gantry.params.top || 15).reverse();
const chart = echarts.init(gantry.root, null, { renderer: 'svg' });
const series = (name, key, color) => ({ name, type: 'bar', stack: 'open', data: people.map(p => p[key]), barMaxWidth: 18, itemStyle: { color, borderColor: gantry.colors.background, borderWidth: 1 } });
chart.setOption({
  aria: { enabled: true },
  textStyle: { color: gantry.colors.text, fontFamily: 'inherit' },
  grid: { left: 170, right: 24, top: 12, bottom: 48 },
  legend: { bottom: 0, textStyle: { color: gantry.colors.text } },
  tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
  xAxis: { type: 'value', minInterval: 1, axisLabel: { color: gantry.colors.muted }, splitLine: { lineStyle: { color: gantry.colors.grid } } },
  // AI agents are marked in the label so a chart never passes one off as a person.
  yAxis: { type: 'category', data: people.map(p => (p.person ? p.person.name + (p.person.kind === 'agent' ? ' (AI agent)' : '') : 'Unassigned')), axisLabel: { color: gantry.colors.text, width: 158, overflow: 'truncate' } },
  series: [series('To do', 'todo', gantry.colors.todo), series('In progress', 'in_progress', gantry.colors.progress), series('Overdue', 'overdue', gantry.colors.danger)].map((s, i) => (i === 2 ? { ...s, stack: 'late' } : s)),
});
gantry.onResize(() => chart.resize());
gantry.cleanup(() => chart.dispose());
