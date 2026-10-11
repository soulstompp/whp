<?php
echo "abc"["x"] ?? "d", "abc"[5] ?? "e", isset("abc"["1x"]) ? "y" : "n", "abc"["1"], "\n";
