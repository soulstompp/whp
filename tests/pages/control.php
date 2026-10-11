<?php
for ($i = 0; $i < 5; $i++) {
    if ($i === 1) continue;
    if ($i === 4) break;
    echo $i;
}
echo "\n";
$i = 0;
while ($i < 3) { echo "w$i"; $i++; }
echo "\n";
do { echo "d$i"; $i--; } while ($i > 0);
echo "\n";
foreach ([1, 2, 3] as $v) {
    if ($v % 2 === 0) { echo "even"; } elseif ($v === 1) { echo "one"; } else { echo "else"; }
}
echo "\n";
for ($i = 0; $i < 3; $i++) {
    for ($j = 0; $j < 3; $j++) {
        if ($j === 1) continue 2;
        if ($i === 2) break 2;
        echo "$i$j ";
    }
}
echo "\n";
echo match(3) { 1, 2 => "low", 3 => "three", default => "other" }, "\n";
echo match(true) { 5 > 3 => "yes", default => "no" }, "\n";
echo match("1") { 1 => "int", "1" => "string", default => "none" }, "\n";
$x = 2;
if ($x > 1): echo "alt-if"; elseif ($x > 0): echo "alt-elseif"; else: echo "alt-else"; endif;
echo "\n";
foreach ([1, 2] as $v): echo $v; endforeach;
for ($i = 0; $i < 2; $i++): echo "f"; endfor;
while ($x-- > 0): echo "w"; endwhile;
echo "\n";
for ($i = 0, $j = 10; $i < 3; $i++, $j--) echo $i + $j;
echo "\n";
if (0) echo "no"; else if (1) echo "else-if"; else echo "no";
echo "\n";
?>
<?php if ($x < 0): ?>
<p>negative</p>
<?php else: ?>
<p>not negative</p>
<?php endif ?>
<?php foreach (["a", "b"] as $k => $v): ?>
<li><?= $k ?>:<?= $v ?></li>
<?php endforeach; ?>
done
