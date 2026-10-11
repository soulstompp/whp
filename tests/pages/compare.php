<?php
function b($v) { return $v ? "T" : "F"; }
echo b(1 == "1"), b("abc" == 0), b(null == false), b("1" == "01"), b("abc" == "ABC"), b(0 == ""), b(null == 0), b("" == null), "\n";
echo b(1 === "1"), b("a" === "a"), b([1, 2] == [1, 2]), b([1, 2] === [2 => 1]), b(["a" => 1, "b" => 2] == ["b" => 2, "a" => 1]), b(["a" => 1, "b" => 2] === ["b" => 2, "a" => 1]), "\n";
echo b("abc" < "abd"), b("10" < "9"), b("10" < "9a"), b(2 < "10"), b("Z" < "a"), b(1 != 2), b(1 <> 1), b(1 !== "1"), "\n";
echo "abc" <=> "abd", " ", [1, 2, 3] <=> [1, 2, 4], " ", null <=> false, " ", [1, 2] <=> [1], "\n";
echo b(!0), b(!"0"), b(!""), b(!"0.0"), b(![]), b(![0]), b(!null), b(!" "), "\n";
echo b(1 && 0), b(1 || 0), b(true xor true), b(null ?? "d"), b(1 and 1), b(0 or 0), "\n";
$x = null;
echo $x ?? "default", " ", $x ?: "elvis", " ", 0 ?: "zero", " ", "set" ?: "no", "\n";
$y = ["k" => null, "j" => 0];
echo $y["k"] ?? "null-key", " ", $y["j"] ?? "zero-key", " ", $y["none"] ?? "missing", "\n";
echo true ? "a" : (false ? "b" : "c"), "\n";
