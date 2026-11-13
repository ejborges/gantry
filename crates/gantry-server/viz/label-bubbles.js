// Labels sized by how much open work carries them. Library: D3 (global `d3`).
const data = await gantry.api.get('/api/v1/analytics/items', { project: gantry.params.project, container: gantry.params.container, group_by: 'label', open: true });
const { width, height } = gantry.size;
const root = d3.pack().size([width, height]).padding(5)(d3.hierarchy({ children: data.groups.slice(0, 60) }).sum(d => d.value));
const svg = d3.select(gantry.root).append('svg').attr('viewBox', [0, 0, width, height]).attr('width', '100%').attr('height', '100%')
  .attr('role', 'img').attr('aria-label', 'Open work by label: ' + data.groups.slice(0, 10).map(g => g.label + ' ' + g.value).join(', '));
if (!data.groups.length) {
  svg.append('text').attr('x', width / 2).attr('y', height / 2).attr('text-anchor', 'middle').attr('fill', gantry.colors.muted).text('No labels on open work');
}
const node = svg.selectAll('g').data(root.leaves()).join('g').attr('transform', d => `translate(${d.x},${d.y})`);
node.append('circle').attr('r', d => d.r).attr('fill', gantry.colors.series[0]);
node.append('title').text(d => d.data.label + ': ' + d.data.value);
node.filter(d => d.r > 18).append('text').attr('text-anchor', 'middle').attr('dy', '-0.1em').attr('fill', '#fff').style('font-size', d => Math.min(14, d.r / 3 + 4) + 'px').text(d => d.data.label.slice(0, Math.floor(d.r / 4)));
node.filter(d => d.r > 18).append('text').attr('text-anchor', 'middle').attr('dy', '1.1em').attr('fill', '#fff').style('font-size', '11px').text(d => d.data.value);
