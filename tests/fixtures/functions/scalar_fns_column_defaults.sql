-- Each storable new function in a DEFAULT: the value written, and the default read
-- back as 18.6 prints it.
-- @setup
CREATE TABLE cd (id INT,
  l TEXT DEFAULT left('abcdef', 2),
  r TEXT DEFAULT right('abcdef', 2),
  s TEXT DEFAULT substr('abcdef', 2, 3),
  o INT DEFAULT octet_length('héllo'),
  m TEXT DEFAULT md5('a'),
  md INT DEFAULT mod(7, 3));
INSERT INTO cd (id) VALUES (1);
-- @test
SELECT id, l, r, s, o, m, md FROM cd;
