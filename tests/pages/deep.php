<?php
function depth($n) { return $n === 0 ? 0 : 1 + depth($n - 1); }
echo depth(100000), "\n";
function down($n) {
    if ($n > 0) {
        down($n - 1);
    } else {
        echo "bottom\n";
    }
}
down(50000);
function ping($n) { return $n === 0 ? "ping" : pong($n - 1); }
function pong($n) { return $n === 0 ? "pong" : ping($n - 1); }
echo ping(99999), "\n";
