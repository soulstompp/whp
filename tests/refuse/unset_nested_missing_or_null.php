<?php
$x = ["a" => 1, "n" => null];
unset($x["missing"][0]);
unset($x["n"][0]);
echo count($x), "\n";
