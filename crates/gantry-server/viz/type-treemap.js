// How the work divides by type, as a treemap. Library: Apache ECharts (global `echarts`).
const data = await gantry.api.get('/api/v1/analytics/items', { project: gantry.params.project, container: gantry.params.container, group_by: 'type' });
// Seven types keep a color of their own, in a fixed order; any further ones are shown together.
const sorted = data.groups.slice().sort((a, b) => b.value - a.value);
const rest = sorted.slice(7);
const shown = sorted.slice(0, 7).concat(rest.length ? [{ label: 'Other types', value: rest.reduce((n, g) => n + g.value, 0), open: rest.reduce((n, g) => n + g.open, 0) }] : []);
const chart = echarts.init(gantry.root, null, { renderer: 'svg' });
chart.setOption({
  aria: { enabled: true },
  textStyle: { fontFamily: 'inherit' },
  tooltip: { formatter: p => p.name + ': ' + p.value + ' (' + p.data.open + ' open)' },
  series: [{
    type: 'treemap', roam: false, nodeClick: false, breadcrumb: { show: false }, left: 4, right: 4, top: 4, bottom: 4,
    label: { show: true, formatter: '{b}\n{c}', fontSize: 13 },
    itemStyle: { borderColor: gantry.colors.background, borderWidth: 2, gapWidth: 2 },
    data: shown.map((g, i) => ({ name: g.label, value: g.value, open: g.open, itemStyle: { color: gantry.colors.series[i] }, label: { color: gantry.colors.seriesInk[i] } })),
  }],
});
gantry.onResize(() => chart.resize());
gantry.cleanup(() => chart.dispose());
