<?php
$a = [-5 => 'a'];
$a[] = 'b';
echo implode(',', array_keys($a)), "\n";
