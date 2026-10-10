-- A RESTRICT child that the SAME delete also cascade-removes must NOT block. Here
-- grandparent G is the parent of both A (CASCADE) and C (CASCADE); C also
-- references A via ON DELETE RESTRICT. Deleting G cascades to A and C — and because
-- C is a direct cascade child of G, it is already in the delete closure when A's
-- RESTRICT edge to C is checked, so the delete succeeds (everything is removed),
-- matching Postgres. (This is the "child deleted together with its referenced
-- parent" case Postgres permits; contrast a child reachable only via a deeper
-- sibling cascade, which Postgres — and pglet — reject.)
-- @setup
CREATE TABLE rcu_g (id INT PRIMARY KEY);
CREATE TABLE rcu_a (id INT PRIMARY KEY, g_id INT REFERENCES rcu_g(id) ON DELETE CASCADE);
CREATE TABLE rcu_c (
  id INT PRIMARY KEY,
  g_id INT REFERENCES rcu_g(id) ON DELETE CASCADE,
  a_id INT REFERENCES rcu_a(id) ON DELETE RESTRICT
);
INSERT INTO rcu_g VALUES (1);
INSERT INTO rcu_a VALUES (100, 1);
INSERT INTO rcu_c VALUES (300, 1, 100);
DELETE FROM rcu_g WHERE id = 1;
-- @test
SELECT
  (SELECT count(*) FROM rcu_g) AS g,
  (SELECT count(*) FROM rcu_a) AS a,
  (SELECT count(*) FROM rcu_c) AS c;
