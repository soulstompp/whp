<?php
function f($a, $b = $a) { return $b; } // refused here
echo f(1), "\n";
