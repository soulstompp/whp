<?php
$x = ["a" => false];
unset($x["a"][0]); // refused here
echo "ok\n";
