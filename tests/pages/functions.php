<?php
function greet($who, $greeting = "Hello") {
    return "$greeting, $who!";
}
echo greet("you"), "\n", greet("them", "Hi"), "\n";
function fact($n) { return $n <= 1 ? 1 : $n * fact($n - 1); }
echo fact(20), "\n";
function fib($n) { return $n < 2 ? $n : fib($n - 1) + fib($n - 2); }
echo fib(20), "\n";
echo later(), "\n";
function later() { return "hoisted"; }
function noreturn() { echo "side "; }
$r = noreturn();
echo $r === null ? "null" : "not", "\n";
function scope() { return isset($outer) ? "sees" : "own scope"; }
$outer = 1;
echo scope(), "\n";
function modify($arr) { $arr[] = 4; return count($arr); }
$arr = [1, 2, 3];
echo modify($arr), count($arr), "\n";
function GREET2() { return "case"; }
echo greet2(), "\n";
function early($n) {
    if ($n > 2) {
        return "big";
    }
    foreach ([1, 2, 3] as $v) {
        if ($v === $n) return "found $v";
    }
    return "none";
}
echo early(5), " ", early(2), " ", early(0), "\n";
function defaults($a, $b = [1, 2], $c = null) { return count($b) . ($c ?? "nc") . $a; }
echo defaults("x"), " ", defaults("y", [], "c"), "\n";
function nested_calls($n) { return strlen(str_repeat("ab", $n)) + count(array_keys([$n, $n])); }
echo nested_calls(3), "\n";
