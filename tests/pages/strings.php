<?php
$name = "World";
$a = ['k' => 'value', 3 => 'three'];
echo "Hello $name!\n";
echo "Braced {$name}s\n";
echo "Array {$a['k']} and $a[k] and $a[3]\n";
echo 'No $name here\n', "\n";
echo "Escapes: \t|\\|\$|\"|\x41|\101|\u{263A}|\e|\v|\f|\n";
echo "Unknown escape \q stays\n";
echo 'single \' and \\ and \n', "\n";
$h = <<<EOT
Heredoc $name
  indented {$a['k']}
EOT;
echo $h, "\n";
$n = <<<'EOT'
Nowdoc $name {$a['k']}
EOT;
echo $n, "\n";
$closing = <<<EOT
    indented closing
      keeps relative
    EOT;
echo $closing, "|\n";
echo strlen("héllo"), "\n";
$s = "hello";
echo $s[0], $s[-1], $s[1], "\n";
echo "{$s} $s[0]\n";
