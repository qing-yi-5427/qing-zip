"""Pure fake-process ownership regression; never enumerate or signal OS processes."""
import ast,types,unittest
from pathlib import Path
class Gone(Exception):pass
class Zombie(Gone):pass
source=Path(__file__).with_name('compare-final-desktops.py')
nodes=[n for n in ast.parse(source.read_text(encoding='utf-8')).body if isinstance(n,ast.FunctionDef) and n.name in ('owned_tree','remember')]
namespace={'psutil':types.SimpleNamespace(NoSuchProcess=Gone,ZombieProcess=Zombie)}
exec(compile(ast.Module(body=nodes,type_ignores=[]),str(source),'exec'),namespace)
class Fake:
 def __init__(self,pid,created,children=(),running=True,vanish=False,reuse_after=False):
  self.pid=pid;self.created=created;self.child=list(children);self.running=running;self.vanish=vanish;self.reuse_after=reuse_after;self.calls=0
 def create_time(self):
  if self.vanish:raise Gone()
  return self.created
 def is_running(self):return self.running
 def children(self,recursive=False):
  assert recursive is False
  self.calls+=1
  if self.vanish:raise Gone()
  if self.reuse_after:self.running=False
  return self.child
 def terminate(self):raise AssertionError('No process termination is permitted in fake test')
class Ownership(unittest.TestCase):
 def ids(self,p):return [(x.pid,x.created) for x in namespace['owned_tree'](p)]
 def test_immediate_parent_time_not_just_root(self):
  old=Fake(3,15);legit=Fake(4,21);parent=Fake(2,20,[old,legit]);root=Fake(1,10,[parent])
  self.assertEqual(self.ids(root),[(1,10),(2,20),(4,21)])
 def test_reused_parent_identity_is_not_traversed(self):
  child=Fake(3,40);stale=Fake(2,20,[child],reuse_after=True)
  self.assertEqual(self.ids(Fake(1,10,[stale])),[(1,10)])
  self.assertEqual(child.calls,0)
 def test_same_pid_different_creation_is_not_same_identity(self):
  a=Fake(2,20,running=False);b=Fake(2,30)
  self.assertEqual(self.ids(Fake(1,10,[a,b])),[(1,10),(2,30)])
 def test_disappearing_child_does_not_own_descendants(self):
  child=Fake(3,40);gone=Fake(2,20,[child],vanish=True)
  self.assertEqual(self.ids(Fake(1,10,[gone])),[(1,10)])
  self.assertEqual(child.calls,0)
 def test_cycle_and_duplicate_edges_are_bounded(self):
  root=Fake(1,10);child=Fake(2,10);root.child=[child,child];child.child=[root]
  self.assertEqual(self.ids(root),[(1,10),(2,10)])
  self.assertEqual(root.calls,1);self.assertEqual(child.calls,1)
 def test_remember_keys_include_creation_and_exclude_old_edge(self):
  owned={};namespace['remember'](Fake(1,10,[Fake(2,9),Fake(3,11)]),owned)
  self.assertEqual(set(owned),{(1,10),(3,11)})
if __name__=='__main__':unittest.main()
