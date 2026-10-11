<?php
function down($n, $trail) {
    if ($n === 0) {
        usleep(1000);
        return "bottom after " . $trail;
    }
    return down($n - 1, $n === 500 ? "the middle" : $trail);
}
echo down(1000, "the top"), "\n";
$total = 0;
foreach ([3, 1, 2] as $i => $v) {
    usleep(500);
    $total += $v;
    echo $i, ":", $v, " ";
}
echo "\n", $total, "\n";
echo sleep(0), "[", usleep(0), "][", usleep(1), "]\n";
function grow_while_sleeping($a) {
    usleep(1);
    $a["b"] = "made " . "after a wait";
    return $a;
}
$arr = ["a" => 1];
$r = grow_while_sleeping($arr);
echo count($arr), count($r), " ", $r["b"], "\n";
?>
<p>after the waits</p>
