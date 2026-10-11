<?php
function f($a = [1, 2][0], $b = 1 + 2 . "x", $c = true ? "y" : "n", $d = null ?? 3) {
    return "$a $b $c $d";
}
echo f(), "\n";
