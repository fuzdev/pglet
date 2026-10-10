-- Two rows of one `INSERT` that are one `jsonb` key — `1` and `1.0` — are a duplicate
-- within the statement, before either is stored.
-- @profile-lock
-- @setup
CREATE TABLE cks_j (j JSONB PRIMARY KEY, n INT);
-- @test
-- @expect-error: an in-statement scale-folded jsonb duplicate (23505)
-- @expect-message
INSERT INTO cks_j (n, j) VALUES (1, '1'), (2, '1.0');
