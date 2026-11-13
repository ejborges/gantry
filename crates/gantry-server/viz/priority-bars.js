// Open work by priority. Library: Apache ECharts (global `echarts`).
const data = await gantry.api.get('/api/v1/analytics/items', { project: gantry.params.project, container: gantry.params.container, group_by: 'priority', open: true });
const names = ['None', 'Low', 'Medium', 'High', 'Critical'];
const values = names.map((_, i) => (data.groups.find(g => g.key === String(i)) || { value: 0 }).value);
const chart = echarts.init(gantry.root, null, { renderer: 'svg' });
chart.setOption({
  aria: { enabled: true },
  grid: { left: 70, right: 24, top: 12, bottom: 28 },
  textStyle: { color: gantry.colors.text, fontFamily: 'inherit' },
  xAxis: { type: 'value', minInterval: 1, axisLabel: { color: gantry.colors.muted }, splitLine: { lineStyle: { color: gantry.colors.grid } } },
  yAxis: { type: 'category', data: names, axisLabel: { color: gantry.colors.text }, axisLine: { lineStyle: { color: gantry.colors.grid } } },
  tooltip: { trigger: 'axis', axisPointer: { type: 'shadow' } },
  series: [{ type: 'bar', data: values.map((v, i) => ({ value: v, itemStyle: { color: gantry.colors.priority[i], borderRadius: [0, 4, 4, 0] } })), barMaxWidth: 22, label: { show: true, position: 'right', color: gantry.colors.text } }],
});
gantry.onResize(() => chart.resize());
gantry.cleanup(() => chart.dispose());
