<?php
$x = ["" => 1];
echo isset($x[null]) ? "y" : "n", "\n"; // refused here
