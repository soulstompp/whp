<?php
$x = ["a" => 5];
unset($x["a"][0]); // refused here
echo "ok\n";
