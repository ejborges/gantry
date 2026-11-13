// How long work takes: days from creation to resolution, one dot per item. Library: Plotly (global `Plotly`).
const c = await gantry.api.get('/api/v1/analytics/cycle-time', { project: gantry.params.project, container: gantry.params.container, days: gantry.params.days || 90 });
const el = gantry.root.appendChild(document.createElement('div'));
el.style.cssText = 'width:100%;height:100%';
const line = (y, name, dash) => ({ x: c.items.length ? [c.items[0].resolved_at, c.items[c.items.length - 1].resolved_at] : [], y: [y, y], mode: 'lines', name, line: { color: gantry.colors.muted, dash, width: 1 }, hoverinfo: 'skip' });
Plotly.newPlot(el, [
  { x: c.items.map(i => i.resolved_at), y: c.items.map(i => i.days), text: c.items.map(i => i.key + ' ' + i.title), mode: 'markers', type: 'scatter', name: 'Resolved items',
    marker: { color: gantry.colors.accent, size: 8, opacity: 0.75 }, hovertemplate: '%{text}<br>%{y:.1f} days<extra></extra>' },
  line(c.stats.median, 'Median ' + c.stats.median + ' d', 'dash'),
  line(c.stats.p85, '85% within ' + c.stats.p85 + ' d', 'dot'),
], {
  margin: { l: 44, r: 12, t: 8, b: 36 }, paper_bgcolor: 'rgba(0,0,0,0)', plot_bgcolor: 'rgba(0,0,0,0)',
  font: { color: gantry.colors.text, family: 'inherit' }, showlegend: true, legend: { orientation: 'h', y: -0.18 },
  xaxis: { gridcolor: gantry.colors.grid }, yaxis: { title: 'Days', rangemode: 'tozero', gridcolor: gantry.colors.grid },
}, { displayModeBar: false, responsive: true });
gantry.onResize(() => Plotly.Plots.resize(el));
gantry.cleanup(() => Plotly.purge(el));
