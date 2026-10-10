-- **Over-rejection guard on the index-probe gate.** pglet stores `varchar` as
-- `Value::Text`, so `text` vs `varchar` is unreachable at eval time — but the
-- probe gate compares the bound value's type against the key column's *declared*
-- `TypeOid`, where the pair is real. Dropping the string family from the accept
-- predicate makes this bound lookup fail `42883` on the accelerated profile while
-- the stripped one returns the row: an over-rejection *and* a profile divergence.
-- @profile-lock
-- @setup
CREATE TABLE vc_probe (v VARCHAR(10), n INT);
CREATE INDEX vc_probe_v ON vc_probe (v);
INSERT INTO vc_probe VALUES ('abc', 1), ('def', 2);
-- @param $1 text abc
-- @test
SELECT n FROM vc_probe WHERE v = $1;
