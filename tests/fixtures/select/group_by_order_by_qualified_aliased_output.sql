-- @setup
CREATE TABLE pa (id INT, x INT);
CREATE TABLE pb (id INT, x INT);
INSERT INTO pa VALUES (1, 10), (2, 20), (3, 30);
INSERT INTO pb VALUES (1, 3), (2, 2), (3, 1);

-- A qualified ORDER BY key over an output column the select list *renamed*. The
-- Aggregate's output column is `k`, so nothing there is spelled `x` for a bare-name
-- resolver to reach — the key has to be resolved by target-entry identity (`pa.x`
-- denotes the projection entry aliased `k`) and rewritten to `k`. The sibling
-- `pb.x` exists and runs the opposite way, so a qualifier-blind identity test would
-- be visible here too.
-- @test
SELECT pa.x AS k FROM pa JOIN pb ON pa.id = pb.id GROUP BY pa.x ORDER BY pa.x DESC;
