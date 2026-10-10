-- A declared name is the user's and is kept as written; a chosen one steps around it. The
-- declared `CHECK` here carries exactly the name 18.6 would choose for the table's next
-- unnamed two-column `CHECK`, so that one takes the digit — the comparison is against the
-- **cut** candidate, which a whole `<table>_check` never equals.
-- @setup
CREATE TABLE cac6_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt (a INT, b INT, CONSTRAINT cac6_tttttttttttttttttttttttttttttttttttttttttttttttttttt_check CHECK (a > 0));
ALTER TABLE cac6_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD CHECK (a > b);
ALTER TABLE cac6_ttttttttttttttttttttttttttttttttttttttttttttttttttttttt ADD CHECK (b > a);
-- @test
SELECT contype::text AS contype, conname::text AS conname, length(conname::text) AS len
FROM pg_constraint WHERE conname LIKE 'cac6\_%' AND contype <> 'n' ORDER BY contype, conname;
