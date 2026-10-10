-- A literal compared with a scalar subquery whose projection is itself a scalar subquery
-- over an enum column is read as that enum, so a label the enum lacks is `22P02`, not
-- compared as `text`. The correlated spelling is
-- `enum/correlated_target_list_nested_subquery_takes_enclosing_enum_loud`.
-- @setup
CREATE TYPE eid_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE eid_te (m eid_mood, k INT);
CREATE TABLE eid_t2 (z TEXT, y TEXT);
INSERT INTO eid_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO eid_t2 VALUES ('a', 'b');
-- @expect-error: 22P02 invalid input value for enum
-- @test
SELECT k FROM eid_te o
 WHERE 'zzz' IN (SELECT (SELECT m FROM eid_te WHERE k = 1) FROM eid_t2 b) ORDER BY k;
