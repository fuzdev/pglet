-- No-regression guard for the missing-FROM-clause fix: valid qualified references
-- across a three-table left-deep join still resolve (`u.*`/`o.*`/`r.*` via the
-- alias-aware tiers), a correlated subquery's outer reference still binds
-- (`o2.user_id = u.id`, resolved against the EvalEnv outer frame — never the
-- removed loose fallback), and `ORDER BY u.id` still resolves against the
-- projected `id` (the no-identity carve-out that keeps `ORDER BY alias.col`
-- working). Every qualifier here names a real in-scope relation, so none is a
-- `42P01`.
-- @setup
CREATE TABLE users (id INT, name TEXT);
CREATE TABLE orders (id INT, user_id INT, total INT);
CREATE TABLE regions (id INT, label TEXT);
INSERT INTO users VALUES (1, 'alice'), (2, 'bob');
INSERT INTO orders VALUES (10, 1, 100), (11, 2, 200);
INSERT INTO regions VALUES (1, 'north'), (2, 'south');

-- @test
SELECT u.id, u.name, o.total, r.label,
       (SELECT COUNT(*) FROM orders o2 WHERE o2.user_id = u.id) AS order_count
FROM users u
JOIN orders o ON o.user_id = u.id
JOIN regions r ON r.id = u.id
ORDER BY u.id;
