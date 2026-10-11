<?php
$title = "Orders";
$rows = [
    ["id" => 1, "who" => "Ana & Bo", "total" => 1250, "paid" => true],
    ["id" => 2, "who" => "<script>", "total" => 99, "paid" => false],
    ["id" => 3, "who" => "Zé", "total" => 100000, "paid" => true],
];
function money($cents) {
    $c = $cents % 100;
    return intdiv($cents, 100) . "." . ($c < 10 ? "0$c" : $c);
}
include __DIR__ . "/inc/header.php";
?>
<table>
<?php foreach ($rows as $i => $row): ?>
  <tr class="<?= $i % 2 ? "odd" : "even" ?>">
    <td><?= $row["id"] ?></td>
    <td><?= htmlspecialchars($row["who"]) ?></td>
    <td><?= money($row["total"]) ?></td>
    <td><?php if ($row["paid"]): ?>paid<?php else: ?>due<?php endif; ?></td>
  </tr>
<?php endforeach; ?>
</table>
<p><?= count($rows) ?> orders, <?= str_repeat("*", 3) ?></p>
<?php include __DIR__ . "/inc/footer.php";
