
#top = array_indexing::index_array

from spade import SpadeExt
from cocotb import cocotb
from cocotb.triggers import Timer

@cocotb.test()
async def test(dut):
    s = SpadeExt(dut)
    s.i.a = "[0, 1, 2, 3]"
    for i in range(0, 4):
        s.i.i = f"{i}u";
        await Timer(1, units='ns')
        s.o.assert_eq(f"{i}u")

