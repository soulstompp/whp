<?php
$s = 'abc';
unset($s[0]); // refused here
echo $s, "\n";
