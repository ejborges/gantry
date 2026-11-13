// Work created per day over the last months, as a calendar heat map. Library: Apache ECharts.
const days = gantry.params.days || 180;
const t = await gantry.api.get('/api/v1/analytics/timeline', { project: gantry.params.project, container: gantry.params.container, interval: 'day', periods: days });
const max = Math.max(1, ...t.periods.map(p => p.created));
const chart = echarts.init(gantry.root, null, { renderer: 'svg' });
chart.setOption({
  aria: { enabled: true },
  textStyle: { fontFamily: 'inherit' },
  tooltip: { formatter: p => gantry.formatDate(p.value[0], { dateStyle: 'medium' }) + ': ' + p.value[1] + ' created' },
  visualMap: { min: 0, max, show: false, inRange: { color: [gantry.colors.grid, gantry.colors.accent] } },
  calendar: {
    range: [t.periods[0].start, t.periods[t.periods.length - 1].start], cellSize: ['auto', 14], left: 36, right: 12, top: 24, bottom: 8,
    itemStyle: { color: 'transparent', borderColor: gantry.colors.background, borderWidth: 2 }, splitLine: { show: false },
    dayLabel: { color: gantry.colors.muted, firstDay: 0 }, monthLabel: { color: gantry.colors.muted }, yearLabel: { show: false },
  },
  series: [{ type: 'heatmap', coordinateSystem: 'calendar', data: t.periods.map(p => [p.start, p.created]) }],
});
gantry.onResize(() => chart.resize());
gantry.cleanup(() => chart.dispose());
