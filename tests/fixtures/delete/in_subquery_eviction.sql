-- Subquery-keyed eviction DELETE (the spine's session/token "keep newest N" shape):
-- DELETE … WHERE id IN (SELECT id … ORDER BY created_at DESC OFFSET n). Without
-- subquery-capable DELETE-filter evaluation the predicate would silently match no
-- rows; without the non-projected ORDER BY fix the inner SELECT would evict by
-- insertion order. created_at order differs from insertion order, so the survivor
-- set distinguishes both bugs. Survivors are compared (deterministic) rather than
-- RETURNING (whose row order is unspecified).
-- @setup
CREATE TABLE auth_session (id INT, account_id INT, created_at INT);
INSERT INTO auth_session VALUES (1, 7, 100), (2, 7, 300), (3, 7, 200), (4, 8, 150);
DELETE FROM auth_session
WHERE id IN (
  SELECT id FROM auth_session WHERE account_id = 7
  ORDER BY created_at DESC OFFSET 1
);

-- @test
SELECT id, account_id, created_at FROM auth_session ORDER BY id;
