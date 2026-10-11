<?php
function f($b = strlen("x")) { return $b; } // refused here
echo f(), "\n";
