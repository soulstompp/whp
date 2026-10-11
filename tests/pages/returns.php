<?php
function make($n) {
    $out = [];
    for ($i = 0; $i < $n; $i++) {
        $out["k" . $i] = ["n" => $i, "s" => str_repeat("x", $i)];
    }
    return $out;
}
$m = make(3);
foreach ($m as $k => $v) echo $k, ":", $v["n"], $v["s"], " ";
echo "\n";
function grow($a) {
    $a[] = "added " . count($a);
    $a["key" . count($a)] = [strlen("made")];
    return $a;
}
$g = grow([1, 2]);
echo implode(",", array_keys($g)), " ", $g[2], " ", $g["key3"][0], "\n";
$orig = ["a" => "b"];
$same = grow($orig);
echo count($orig), count($same), " ", implode(",", array_keys($same)), "\n";
function pick($a, $k) { return $a[$k]; }
echo pick(["x" => "y" . "z"], "x"), "\n";
function nested($d) { return $d === 0 ? ["leaf" . $d] : [nested($d - 1), "level" . $d]; }
$t = nested(3);
echo $t[1], $t[0][1], $t[0][0][1], $t[0][0][0][0], "\n";
function word() { return "made " . "inside"; }
$w = [word() => word()];
echo implode("|", array_keys($w)), "=", $w[word()], "\n";
function keep($a) { return $a; }
$k1 = ["p" => "q"];
$k2 = keep($k1);
$k2["p"] = "changed";
echo $k1["p"], $k2["p"], "\n";
