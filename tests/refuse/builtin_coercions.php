<?php
echo htmlspecialchars(123), htmlspecialchars(true), str_repeat("ab", "2"), str_repeat("ab", true), intdiv("10", "3"), intdiv(" 10", 3), "\n";
echo is_int("5") ? "y" : "n", empty(strlen('')) ? "y" : "n", "\n";
