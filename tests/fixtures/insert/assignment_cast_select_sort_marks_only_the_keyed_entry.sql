-- **A false-rejection control — do not trim it.** `addTargetToSortList` coerces the
-- entry a sort key **names** and no other, so `ORDER BY 2` resolves the second entry's
-- `unknown` and leaves the first one's alone. Measured on 18.6: this statement is
-- accepted while its `ORDER BY 1` spelling is `42804`.
--
-- It is the fixture that says a resolving node marks **positions** rather than its
-- whole output list. Mark the whole list and this one turns into a spurious `42804` on
-- column `b`, whose entry no key names — while every single-entry fixture in this set
-- stays green, which is why the second column is the point.
-- @setup
CREATE TABLE acsm (id INT, b BOOL, i INT);
-- @test
INSERT INTO acsm (b, i) SELECT 't', 5 ORDER BY 2 RETURNING b, i;
