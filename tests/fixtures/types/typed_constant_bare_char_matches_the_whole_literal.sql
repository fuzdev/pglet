-- The silent-wrong shape the truncation had in a **read**: a `WHERE` over a parenless
-- `char` constant compared the first character alone, so it selected the row that
-- happens to hold that prefix and not the row the statement named. Both rows are
-- present so a fixture that lost the rule answers `2` rather than failing to answer.
-- @setup
CREATE TABLE cwm (id INT, c TEXT);
INSERT INTO cwm VALUES (1, 'ab'), (2, 'a');
-- @test
SELECT id FROM cwm WHERE c = char 'ab' ORDER BY id;
