<p>before</p>
<?php echo "one"; ?>
<p>after one</p>
<?php echo "two" ?>

<p>the blank line above stays</p>
<?= "three" ?>
<?= "four", "five" ?>
<? echo "short"; ?>
<?php
echo "multi\n";
?>
<?php echo "a"; ?><?php echo "b"; ?>
<?php echo '?> in a string', "\n"; ?>
<?php echo "x" ?> trailing text
<?php // a comment ends at ?> so this is HTML
<?php # a hash comment ?>
<?php /* a block comment ?> holds the tag */ echo "after block\n"; ?>
<p>end</p>
<?php echo "no close at end\n";
