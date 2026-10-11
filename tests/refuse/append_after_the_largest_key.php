<?php
$a = [PHP_INT_MAX => 1];
$a[] = 2; // refused here
echo count($a), "\n";
