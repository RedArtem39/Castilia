// libs: moment.js dayjs.js luxon.js
// expect: 2020-01-02|2020-01-03|2020-01-04
console.log([moment('2020-01-01').add(1,'day').format('YYYY-MM-DD'), dayjs('2020-01-01').add(2,'day').format('YYYY-MM-DD'), luxon.DateTime.fromISO('2020-01-01').plus({days:3}).toISODate()].join('|'));
