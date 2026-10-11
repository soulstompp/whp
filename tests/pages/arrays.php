<?php
$a = [1, 2, 3];
$a[] = 4;
$b = $a;
$b[] = 5;
echo count($a), " ", count($b), "\n";
$m = ["8" => "eight", "08" => "zero-eight", 1 => "one", true => "true-key", "" => "empty-key", "x" => "ex"];
foreach ($m as $k => $v) {
    echo is_int($k) ? "int " : "str ", $k, " => ", $v, "\n";
}
$n = [5 => "a", "b", 10 => "c", "d"];
foreach ($n as $k => $v) echo $k, $v, " ";
echo "\n";
$neg = [-5 => "a", "b"];
foreach ($neg as $k => $v) echo $k, $v, " ";
echo "\n";
$nested = ["rows" => [["id" => 1, "name" => "A"], ["id" => 2, "name" => "B"]]];
$nested["rows"][] = ["id" => 3, "name" => "C"];
$nested["rows"][0]["name"] = "Z";
foreach ($nested["rows"] as $row) echo $row["id"], $row["name"], ";";
echo "\n";
$c = [1, 2, 3];
foreach ($c as $v) { $c[] = $v * 10; }
echo implode(",", $c), "\n";
$keys = array_keys(["a" => 1, "b" => 2, 7 => 3]);
echo implode(",", $keys), "\n";
echo in_array("2", [1, 2, 3]) ? "y" : "n", in_array("2", [1, 2, 3], true) ? "y" : "n", in_array("abc", [0]) ? "y" : "n", "\n";
unset($c[0]);
$c[] = 99;
foreach ($c as $k => $v) echo "$k=$v ";
echo "\n";
echo isset($m["x"]) ? "set" : "unset", " ", isset($m["nope"]) ? "set" : "unset", " ", empty($m["08"]) ? "empty" : "full", " ", empty($m["nope"]) ? "empty" : "full", "\n";
$fresh[] = "auto";
$fresh["k"]["deep"] = "made";
echo count($fresh), $fresh[0], $fresh["k"]["deep"], "\n";
echo implode(" ", ["a", 1, true, null, "b"]), "|", implode([1, 2]), "\n";
$copy = $nested;
$copy["rows"][1]["name"] = "changed";
echo $nested["rows"][1]["name"], $copy["rows"][1]["name"], "\n";
echo count([]), count([[1, 2], [3]]), "\n";
