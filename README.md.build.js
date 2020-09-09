var vfile = require('to-vfile');
var remark = require('remark');
var graphviz = require('remark-graphviz');
var include = require('remark-code-import');
 
var example = vfile.readSync('README.src.md');
 
remark()
  .use(include)
  .use(graphviz)
  .process(example, function (err, contents) {
    if (err) throw err;
    vfile.writeSync({ path: 'README.md', contents });
    console.log("DONE");
  });
