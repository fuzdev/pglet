-- ...and the `IF NOT EXISTS` skip precedes both, so the bad owner is never resolved
-- at all: the statement is a plain NOTICE and the existing sequence survives.
-- @setup
CREATE TABLE ine_prec_host (a INT);
CREATE SEQUENCE ine_prec_seq START WITH 9;
CREATE SEQUENCE IF NOT EXISTS ine_prec_seq OWNED BY ine_prec_host.nosuch;
-- @test
SELECT nextval('ine_prec_seq');
