<?php
$title = "Included";
include __DIR__ . "/inc/header.php";
$config = require __DIR__ . "/inc/config.php";
echo "site: ", $config["site"], "\n";
echo include_once __DIR__ . "/inc/once.php", "\n";
echo (include_once __DIR__ . "/inc/once.php") === true ? "already" : "again", "\n";
echo (require_once __DIR__ . "/inc/once.php") === true ? "already" : "again", "\n";
echo (include __DIR__ . "/inc/plain.php"), "\n";
function inner() {
    $local = "inner-local";
    include __DIR__ . "/inc/uses_local.php";
    return $made;
}
echo inner(), "\n";
echo isset($made) ? "leaked" : "kept inside", "\n";
include __DIR__ . "/inc/defines.php";
echo from_include(), "\n";
include __DIR__ . "/inc/footer.php";
