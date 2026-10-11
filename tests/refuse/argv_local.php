<?php
function f() {
    $argv = 'mine'; // refused here
    return $argv;
}
echo f(), "\n";
