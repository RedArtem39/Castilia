// libs: jquery.js
// expect: jq 3 li-2 clicked done
$('body').append('<ul id=l><li>a</li><li class=x>b</li><li>c</li></ul>');
let clicked = '';
$('#l li').on('click', function(){ clicked = 'clicked'; });
$('#l .x').trigger('click');
const out = ['jq', $('#l li').length, 'li-' + ($('#l li.x').index() + 1), clicked];
$.Deferred().resolve().then(() => console.log(out.join(' ') + ' done'));
