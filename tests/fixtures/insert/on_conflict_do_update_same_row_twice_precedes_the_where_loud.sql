-- The refusal comes before the action's `WHERE` is evaluated, so an error the second
-- proposal's `WHERE` would raise loses to it: `21000`, not the division's `22012`. The first
-- proposal's `WHERE` divides by one and updates the row.
-- @profile-lock
-- @setup
CREATE TABLE ocsp (id INT PRIMARY KEY, v TEXT);
INSERT INTO ocsp VALUES (1, 'a');

-- @test
-- @expect-error: ON CONFLICT DO UPDATE command cannot affect row a second time (21000), ahead of the WHERE's 22012
INSERT INTO ocsp VALUES (1, 'x'), (1, 'y')
ON CONFLICT (id) DO UPDATE SET v = excluded.v
WHERE 1 / (CASE WHEN excluded.v = 'y' THEN 0 ELSE 1 END) = 1;
