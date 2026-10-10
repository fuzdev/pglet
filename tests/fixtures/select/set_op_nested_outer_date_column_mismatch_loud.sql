-- The inner set operation's own mismatch — the enclosing `date` column beside `8` — is
-- the error, ahead of the outer one's `'2020-01-02'`.
-- @setup
CREATE TABLE sodm (d DATE);
INSERT INTO sodm VALUES ('2020-01-02');
-- @expect-error: EXCEPT types date and integer cannot be matched
-- @test
SELECT count(*) FROM sodm WHERE EXISTS ((SELECT d EXCEPT SELECT 8) UNION SELECT '2020-01-02');
