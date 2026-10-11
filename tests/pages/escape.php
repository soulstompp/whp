<?php
$s = '<a href="x?a=1&b=2">Tom\'s & "quotes"</a>';
echo htmlspecialchars($s), "\n";
echo htmlspecialchars("&amp; already"), "\n";
echo htmlspecialchars("bad \xff byte and \xc3 cut"), "|\n";
echo htmlspecialchars(""), "|\n";
echo htmlspecialchars("Zé ☺ \u{1F600}"), "\n";
echo htmlspecialchars(42), "\n";
