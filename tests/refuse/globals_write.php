<?php
$GLOBALS['x'] = 1; // refused here
echo isset($x) ? 'y' : 'n', "\n";
