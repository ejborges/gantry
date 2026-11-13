// Projects, then types, then status, as nested rings. Library: D3 (global `d3`).
const data = await gantry.api.get('/api/v1/analytics/items', { project: gantry.params.project, container: gantry.params.container, group_by: 'project,type,status_category' });
const tree = { name: 'All', children: [] };
for (const g of data.groups) {
  let at = tree;
  g.path.forEach((part, depth) => {
    let next = at.children.find(c => c.name === part.label);
    if (!next) { next = { name: part.label, key: part.key, depth, children: [] }; at.children.push(next); }
    at = next;
  });
  at.value = g.value;
}
const { width, height } = gantry.size;
const radius = Math.min(width, height) / 2 - 4;
const root = d3.partition().size([2 * Math.PI, radius])(d3.hierarchy(tree).sum(d => d.value || 0));
const status = { todo: gantry.colors.todo, in_progress: gantry.colors.progress, done: gantry.colors.done };
const projects = (root.children || []).map(d => d.data.name);
const color = name => (projects.indexOf(name) < gantry.colors.series.length ? gantry.colors.series[projects.indexOf(name)] : gantry.colors.muted);
const fill = d => (d.depth === 3 ? status[d.data.key] : color((d.depth === 1 ? d : d.parent).data.name));
const arc = d3.arc().startAngle(d => d.x0).endAngle(d => d.x1).innerRadius(d => d.y0).outerRadius(d => d.y1 - 1).padAngle(0.004);
const svg = d3.select(gantry.root).append('svg').attr('viewBox', [-width / 2, -height / 2, width, height]).attr('width', '100%').attr('height', '100%')
  .attr('role', 'img').attr('aria-label', 'Work by project, type and status');
svg.selectAll('path').data(root.descendants().filter(d => d.depth)).join('path').attr('d', arc).attr('fill', fill).attr('fill-opacity', d => (d.depth === 2 ? 0.7 : 1))
  .append('title').text(d => d.ancestors().reverse().slice(1).map(a => a.data.name).join(' › ') + ': ' + d.value);
svg.append('text').attr('text-anchor', 'middle').attr('dy', '0.35em').attr('fill', gantry.colors.text).style('font-size', '20px').style('font-weight', 600).text(root.value);
