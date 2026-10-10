-- The `SET DEFAULT` route's half of the same registration. Its sequence pass read the
-- closed `NextVal` arm too, so an expression default recorded nothing and the column
-- answered `42P01` at every omitting write — a column defined and unwritable, on a
-- statement both engines accept.
-- @setup
CREATE SEQUENCE sdseq_expr;
CREATE TABLE sdseq_t (k INT, v INT);
ALTER TABLE sdseq_t ALTER COLUMN v SET DEFAULT nextval('sdseq_expr') + 1;
INSERT INTO sdseq_t (k) VALUES (1);
INSERT INTO sdseq_t (k) VALUES (2);
-- @test
SELECT k, v FROM sdseq_t ORDER BY k;
