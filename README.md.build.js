var vfile = require('to-vfile');
var remark = require('remark');
var graphviz = require('remark-graphviz');
 
var example = vfile.readSync('README.src.md');
 
remark()
  .use(graphviz)
  .process(example, function (err, contents) {
    if (err) throw err;
 
    console.log(vfile.writeSync({ path: 'README.md', contents }));
  });
