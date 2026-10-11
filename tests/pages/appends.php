<?php
// `.=` building long strings while other values hold copies of them, and the order of a compound
// assignment.
$s = '';
for ($i = 0; $i < 3000; $i++) {
    $s .= 'ab';
}
$t = $s;
for ($i = 0; $i < 3000; $i++) {
    $s .= 'cd';
}
echo strlen($t), ' ', strlen($s), ' ', $t[5999], $s[11999], "\n";
echo $t === str_repeat('ab', 3000) ? 'kept' : 'changed', "\n";
$u = $s;
$s .= $s;
echo strlen($s), ' ', $u === str_repeat('ab', 3000) . str_repeat('cd', 3000) ? 'kept' : 'changed', "\n";
$v = "$s!";
for ($i = 0; $i < 9000; $i++) {
    $s .= 'e';
}
echo strlen($v), ' ', $v[24000], ' ', strlen($s), "\n";
function build($n) {
    $x = '';
    for ($i = 0; $i < $n; $i++) {
        $x .= 'z';
    }
    return $x;
}
$b = build(9000);
$b .= '!';
echo strlen($b), $b[9000], "\n";
$c = 'a';
$c .= ($c = 'y');
echo $c, "\n";
$n = 1;
$n += ($n = 5);
echo $n, ' ', ($m = 'p') . ($m .= 'q'), ' ', $m, "\n";
