-- The same false rejection on the `UPDATE … SET` right-hand side: it was gated against
-- the target relation's scope but never *resolved* against it, so the bare `'10'` typed
-- as `text` and the assignment was rejected as `integer = text`. The `WHERE` on the same
-- statement always resolved first — the assignments were simply handed the gate without
-- the pass that makes the gate right.
-- @setup
CREATE TABLE usl (id INT, i4 INT4, b BOOL);
INSERT INTO usl VALUES (1, 10, false), (2, 20, false);
-- @test
UPDATE usl SET b = (i4 = '10') WHERE id < '3' RETURNING id, b;
