import importlib.util,io,hashlib,tempfile,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('downloader',str(Path(__file__).resolve().parents[1] / 'download-full.py')); m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
class Downloads(unittest.TestCase):
 def fixture(self,corrupt=False):
  name='wincraft-openrc-20261012-x86_64.iso'; chunks=[b'first part',b'second part']; data={'hash':(hashlib.sha256(b''.join(chunks)).hexdigest()+'  '+name).encode(),'one':chunks[0],'two':b'bad' if corrupt else chunks[1]};m.open_url=lambda url:io.BytesIO(data[url]);assets=[{'name':name+'.part01','size':len(chunks[1]),'browser_download_url':'two'},{'name':name+'.sha256','browser_download_url':'hash'},{'name':name+'.part00','size':len(chunks[0]),'browser_download_url':'one'}];return name,[{'assets':assets,'published_at':'2026-10-12'}]
 def test_join_and_verify(self):
  name,releases=self.fixture()
  with tempfile.TemporaryDirectory() as t:
   result=m.download_image(releases,t);self.assertEqual(result.read_bytes(),b'first partsecond part')
   with self.assertRaises(FileExistsError):m.download_image(releases,t)
 def test_truncated_part_removed(self):
  name,releases=self.fixture(True)
  with tempfile.TemporaryDirectory() as t:
   with self.assertRaises(ValueError):m.download_image(releases,t)
   self.assertEqual(list(Path(t).iterdir()),[])
 def test_bad_checksum_removed(self):
  name,releases=self.fixture();original=m.open_url;m.open_url=lambda u:io.BytesIO(b'0'*64) if u=='hash' else original(u)
  with tempfile.TemporaryDirectory() as t:
   with self.assertRaises(ValueError):m.download_image(releases,t)
   self.assertEqual(list(Path(t).iterdir()),[])
 def test_missing_part_rejected(self):
  _,releases=self.fixture();releases[0]['assets'].pop()
  with self.assertRaises(ValueError):m.select_image(releases)
 def test_draft_excluded(self):
  _,releases=self.fixture();releases[0]['draft']=True
  with self.assertRaises(ValueError):m.select_image(releases)
unittest.main()
