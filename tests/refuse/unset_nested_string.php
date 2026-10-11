<?php
$x = ["a" => "str"];
unset($x["a"][0]); // refused here
echo "ok\n";
