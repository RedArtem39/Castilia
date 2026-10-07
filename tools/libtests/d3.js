// libs: d3.js
// expect: d3 3 <svg><circle r="1"></circle><circle r="2"></circle><circle r="3"></circle></svg>
const s = d3.select('body').append('svg'); s.selectAll('circle').data([1,2,3]).enter().append('circle').attr('r', d=>d);
console.log('d3', d3.max([1,3,2]), document.querySelector('svg').outerHTML);
